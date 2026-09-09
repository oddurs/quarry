//! Turning raw sockets into attributed services.
//!
//! The engine owns no I/O of its own — it composes a socket source, a process
//! source and a working-directory fallback. Swap all three for fixtures and the
//! same code path runs with no machine underneath it, which is how the
//! discovery tests work.

use std::collections::BTreeMap;

use crate::diag;
use crate::model::{Kind, Listener, Rules, Server, classify};
use crate::repo::Resolver;
use crate::signature::{Evidence, Registry, Verdict};
use crate::source::{CwdSource, ProcInfo, ProcessSource, SocketSource, SourceError};

/// A hard ceiling on how much one scan can produce. A machine with this many
/// listeners is either under attack or running something pathological; either
/// way, refusing to grow without bound is the right answer.
pub const MAX_SERVERS: usize = 2048;

pub struct Engine {
    sockets: Box<dyn SocketSource>,
    procs: Box<dyn ProcessSource>,
    cwds: Box<dyn CwdSource>,
    repos: Resolver,
    rules: Rules,
    signatures: Registry,
    containers: crate::docker::Containers,
    scans: u64,
}

#[derive(Debug)]
pub struct ScanReport {
    pub servers: Vec<Server>,
    /// Non-fatal problems worth telling the user about.
    pub warnings: Vec<String>,
    pub truncated: bool,
}

impl Engine {
    pub fn new(
        sockets: Box<dyn SocketSource>,
        procs: Box<dyn ProcessSource>,
        cwds: Box<dyn CwdSource>,
    ) -> Self {
        Self {
            sockets,
            procs,
            cwds,
            repos: Resolver::new(),
            rules: Rules::default(),
            signatures: Registry::builtin(),
            containers: Default::default(),
            scans: 0,
        }
    }

    /// Classification rules from the user's config, which win over the
    /// built-in tables.
    pub fn with_rules(mut self, rules: Rules) -> Self {
        self.rules = rules;
        self
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    /// The signature table this engine identifies with.
    pub fn with_signatures(mut self, signatures: Registry) -> Self {
        self.signatures = signatures;
        self
    }

    pub fn signatures(&self) -> &Registry {
        &self.signatures
    }

    /// The real machine.
    ///
    /// Prefers the kernel directly: `lsof` costs about 58ms of a 59ms scan,
    /// because it walks every descriptor of every process and formats the
    /// answer as text for us to parse back. It stays as the fallback, since a
    /// platform without the native path still has to work.
    pub fn live() -> Self {
        Self::new(
            Self::socket_source(),
            Box::new(crate::procs::SysProcesses::new()),
            Box::new(crate::lsof::LsofCwds::default()),
        )
    }

    /// The socket source this machine will actually use. Public so `--doctor`
    /// can report on the one that is live rather than guessing.
    pub fn socket_source() -> Box<dyn SocketSource> {
        #[cfg(target_os = "macos")]
        {
            if crate::darwin::available() {
                return Box::new(crate::darwin::Native);
            }
            diag::warn(
                "engine",
                "the kernel would not enumerate processes; falling back to lsof",
            );
        }
        Box::new(crate::lsof::Lsof)
    }

    pub fn describe(&self) -> (String, String) {
        (self.sockets.describe(), self.procs.describe())
    }

    pub fn scan(&mut self) -> Result<ScanReport, SourceError> {
        self.scans += 1;
        // Repository layout changes rarely but does change — a branch switch
        // or a new worktree should not need a restart to show up.
        if self.scans.is_multiple_of(20) {
            self.repos.clear();
        }

        // Asked once per scan, not once per port.
        self.containers = crate::docker::Containers::query();

        let mut raw = self.sockets.listening()?;
        let mut warnings = Vec::new();

        let truncated = raw.len() > MAX_SERVERS;
        if truncated {
            warnings.push(format!(
                "{} listening sockets found; showing the first {MAX_SERVERS}",
                raw.len()
            ));
            diag::warn("engine", warnings[0].clone());
            raw.truncate(MAX_SERVERS);
        }

        // One process may hold many sockets; group before touching the process
        // table so we ask about each pid exactly once.
        let mut grouped: BTreeMap<u32, (String, String, Vec<Listener>)> = BTreeMap::new();
        for socket in raw {
            let entry = grouped
                .entry(socket.pid)
                .or_insert_with(|| (socket.command.clone(), socket.user.clone(), Vec::new()));
            if entry.0.is_empty() {
                entry.0 = socket.command;
            }
            if entry.1.is_empty() {
                entry.1 = socket.user;
            }
            entry.2.push(socket.listener);
        }

        let pids: Vec<u32> = grouped.keys().copied().collect();
        self.procs.refresh(&pids);

        let mut info: BTreeMap<u32, ProcInfo> = pids
            .iter()
            .map(|pid| (*pid, self.procs.info(*pid).unwrap_or_default()))
            .collect();

        // Fall back to the expensive lookup only for the processes whose
        // working directory we could not otherwise read.
        let missing: Vec<u32> = info
            .iter()
            .filter(|(_, i)| i.cwd.is_none())
            .map(|(pid, _)| *pid)
            .collect();
        if !missing.is_empty() {
            for (pid, cwd) in self.cwds.cwds(&missing) {
                if let Some(i) = info.get_mut(&pid) {
                    i.cwd = Some(cwd);
                }
            }
        }

        let mut servers = Vec::with_capacity(grouped.len());
        for (pid, (command, user, mut listeners)) in grouped {
            let i = info.remove(&pid).unwrap_or_default();

            // TCP first, then UDP, then unix: a service reachable several ways
            // is named after the one people use. Deduped on the whole endpoint,
            // since every unix socket has port zero.
            listeners.sort_by(|a, b| {
                a.transport
                    .cmp(&b.transport)
                    .then(a.port.cmp(&b.port))
                    .then(a.path.cmp(&b.path))
                    .then(a.addr.is_ipv6().cmp(&b.addr.is_ipv6()))
            });
            listeners.dedup_by(|a, b| {
                a.transport == b.transport && a.port == b.port && a.path == b.path
            });
            if listeners.is_empty() {
                continue;
            }

            let command = if command.is_empty() {
                i.name.clone()
            } else {
                command
            };
            let cwd = i.cwd.clone();
            let exe = i.exe.clone();
            // A published port belongs to the runtime as far as the operating
            // system is concerned. The daemon knows whose it really is.
            let container = listeners
                .iter()
                .filter(|l| l.port != 0)
                .find_map(|l| self.containers.get(l.port))
                .cloned();
            // The working directory is the strong signal. The executable's
            // location is a weak one — every Homebrew-installed daemon lives
            // inside Homebrew's own git repository — so it is consulted only
            // when there is no working directory at all, and never for a
            // package manager's prefix.
            // A Compose project names a directory on disk, which resolves to a
            // repository exactly as a process's working directory does — so a
            // container and a process from one repo land in one group.
            let compose_repo = container
                .as_ref()
                .and_then(|c| c.working_dir.as_deref())
                .and_then(|d| self.repos.resolve(d));

            let repo = match cwd.as_deref() {
                Some(c) => self.repos.resolve(c),
                None => exe
                    .as_deref()
                    .and_then(|e| e.parent())
                    .and_then(|d| self.repos.resolve(d))
                    .filter(|r| !is_package_prefix(&r.root)),
            };
            let repo = compose_repo.or(repo);

            let ports: Vec<u16> = listeners
                .iter()
                .filter(|l| l.port != 0)
                .map(|l| l.port)
                .collect();
            let uri_port = ports.first().copied().unwrap_or(0);
            let uri_path = listeners
                .first()
                .and_then(|l| l.path.as_ref())
                .map(|p| p.display().to_string());
            let identified = identify(
                &self.rules,
                &self.signatures,
                &Evidence {
                    command: &command,
                    cmdline: &i.cmdline,
                    ports: &ports,
                    ..Default::default()
                },
            );
            let signature = identified
                .verdict
                .as_ref()
                .and_then(|v| self.signatures.get(v.index));

            servers.push(Server {
                pid,
                ppid: i.ppid,
                command,
                cmdline: i.cmdline,
                exe,
                user,
                cwd,
                listeners,
                repo,
                kind: identified.kind,
                service: identified
                    .verdict
                    .as_ref()
                    .filter(|v| v.names_the_service())
                    .map(|v| v.name.clone()),
                health_path: signature.and_then(|s| s.health.clone()),
                uri: signature.and_then(|s| s.uri_for(uri_port, uri_path.as_deref())),
                note: signature.and_then(|s| s.note.clone()),
                handshake: signature.and_then(|s| s.probe.clone()),
                banner: None,
                evidence: identified
                    .verdict
                    .as_ref()
                    .map(|v| v.reasons.clone())
                    .unwrap_or_default(),
                container,
                started_at: i.started_at,
                cpu: i.cpu,
                mem: i.mem,
                health: Default::default(),
            });
        }

        servers.sort_by(|a, b| {
            a.group_key()
                .cmp(&b.group_key())
                .then(a.primary_port().cmp(&b.primary_port()))
        });

        Ok(ScanReport {
            servers,
            warnings,
            truncated,
        })
    }
}

/// What a service is, and why.
///
/// Three sources of truth, in descending order of how much they know:
/// the user's own `[ports]`/`[names]` rules, which are statements rather than
/// guesses; the signature table; and finally the port-range conventions, which
/// are the last resort and know only that 3000-ish means "some web thing".
pub struct Identified {
    pub kind: Kind,
    pub verdict: Option<Verdict>,
}

pub fn identify(rules: &Rules, signatures: &Registry, evidence: &Evidence) -> Identified {
    if let Some(kind) = rules.classify(evidence.command, evidence.cmdline, evidence.ports) {
        return Identified {
            kind,
            verdict: None,
        };
    }
    if let Some(verdict) = signatures.identify(evidence) {
        return Identified {
            kind: verdict.kind,
            verdict: Some(verdict),
        };
    }
    Identified {
        kind: classify(evidence.command, evidence.cmdline, evidence.ports),
        verdict: None,
    }
}

/// Roots that belong to a package manager rather than to a project.
fn is_package_prefix(root: &std::path::Path) -> bool {
    const PREFIXES: [&str; 6] = [
        "/opt/homebrew",
        "/usr/local",
        "/opt/local",
        "/nix",
        "/home/linuxbrew/.linuxbrew",
        "/var/lib/snapd",
    ];
    PREFIXES
        .iter()
        .any(|p| root == std::path::Path::new(p) || root.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;
    use crate::source::{StaticCwds, StaticProcesses, StaticSockets};
    use crate::testkit;

    fn engine(sockets: Vec<crate::source::RawSocket>) -> Engine {
        Engine::new(
            Box::new(StaticSockets::new(sockets)),
            Box::new(StaticProcesses::default()),
            Box::new(StaticCwds::default()),
        )
    }

    #[test]
    fn one_process_with_many_ports_is_one_service() {
        let mut e = engine(vec![
            testkit::socket(900, "node", 3000),
            testkit::socket(900, "node", 3001),
            testkit::socket(900, "node", 3000),
        ]);
        let report = e.scan().expect("scan succeeds");
        assert_eq!(report.servers.len(), 1, "grouped by pid");
        assert_eq!(
            report.servers[0].listeners.len(),
            2,
            "duplicate ports collapse"
        );
        assert_eq!(report.servers[0].primary_port(), 3000, "lowest port leads");
    }

    #[test]
    fn a_source_failure_is_reported_not_swallowed() {
        let mut e = Engine::new(
            Box::new(StaticSockets::failing("lsof: timed out")),
            Box::new(StaticProcesses::default()),
            Box::new(StaticCwds::default()),
        );
        let err = e.scan().unwrap_err();
        assert!(err.transient, "a timeout is worth retrying");
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn falls_back_to_the_cwd_source_when_the_process_table_is_blind() {
        let mut cwds = StaticCwds::default();
        cwds.cwds.insert(900, std::env::temp_dir());
        let mut e = Engine::new(
            Box::new(StaticSockets::new(vec![testkit::socket(900, "node", 3000)])),
            Box::new(StaticProcesses::default()),
            Box::new(cwds),
        );
        let report = e.scan().expect("scan succeeds");
        assert_eq!(report.servers[0].cwd, Some(std::env::temp_dir()));
    }

    #[test]
    fn refuses_to_grow_without_bound() {
        let sockets: Vec<_> = (0..MAX_SERVERS as u32 + 10)
            .map(|i| testkit::socket(i + 1, "node", 1024 + (i % 60000) as u16))
            .collect();
        let mut e = engine(sockets);
        let report = e.scan().expect("scan succeeds");
        assert!(report.truncated, "oversized scans are flagged");
        assert!(report.servers.len() <= MAX_SERVERS);
        assert!(!report.warnings.is_empty(), "the user is told why");
    }

    #[test]
    fn user_rules_reach_the_scan() {
        let (rules, problems) = crate::model::Rules::from_config(
            &[("5432".to_string(), "web".to_string())].into(),
            &Default::default(),
        );
        assert!(problems.is_empty(), "{problems:?}");
        let mut e = engine(vec![testkit::socket(900, "postgres", 5432)]).with_rules(rules);
        let report = e.scan().expect("scan succeeds");
        assert_eq!(
            report.servers[0].kind,
            Kind::Web,
            "a user rule must beat the built-in table"
        );
    }

    #[test]
    fn classification_survives_an_empty_process_table() {
        let mut e = engine(vec![testkit::socket(900, "postgres", 5432)]);
        let report = e.scan().expect("scan succeeds");
        assert_eq!(report.servers[0].kind, Kind::Database);
    }

    #[test]
    fn a_package_manager_prefix_is_not_a_project() {
        assert!(is_package_prefix(std::path::Path::new("/opt/homebrew")));
        assert!(is_package_prefix(std::path::Path::new("/usr/local")));
        assert!(!is_package_prefix(std::path::Path::new(
            "/Users/x/Code/acme"
        )));
    }

    #[test]
    fn the_working_directory_outranks_the_executable_path() {
        let tmp = std::env::temp_dir().join("quarry-engine-test-cwd");
        std::fs::create_dir_all(&tmp).expect("create dir");
        let mut cwds = StaticCwds::default();
        cwds.cwds.insert(900, tmp.clone());

        let mut procs = std::collections::HashMap::new();
        procs.insert(
            900,
            crate::source::ProcInfo {
                cmdline: "python3 -m http.server".into(),
                name: "python3".into(),
                exe: Some(std::path::PathBuf::from("/opt/homebrew/bin/python3")),
                cwd: Some(tmp.clone()),
                ..Default::default()
            },
        );

        let mut e = Engine::new(
            Box::new(StaticSockets::new(vec![testkit::socket(
                900, "python3", 8000,
            )])),
            Box::new(StaticProcesses::new(procs)),
            Box::new(cwds),
        );
        let report = e.scan().expect("scan succeeds");
        let s = &report.servers[0];
        assert_ne!(
            s.repo.as_ref().map(|r| r.name.as_str()),
            Some("homebrew"),
            "a homebrew binary must not claim the homebrew repository"
        );
        assert_eq!(s.cwd.as_ref(), Some(&tmp));
        let _ = std::fs::remove_dir(&tmp);
    }

    #[test]
    fn an_empty_machine_is_an_empty_result_not_an_error() {
        let mut e = engine(vec![]);
        let report = e.scan().expect("an idle machine is not a failure");
        assert!(report.servers.is_empty());
        assert!(report.warnings.is_empty());
    }
}
