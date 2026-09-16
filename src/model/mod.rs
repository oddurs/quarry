//! Types describing a listening service.
//!
//! Split by the question each part answers: `kind` what it is, `health`
//! whether it is answering, `listener` where, `project` whose it is, `rules`
//! what the user has said about their own machine, and `view` how the list is
//! arranged. `Server` lives here because it is the thing they all describe.

mod health;
mod kind;
mod listener;
mod project;
mod rules;
mod view;

pub use health::{Health, fmt_ms, status_text};
pub use kind::{Kind, classify, classify_with, friendly_process, refine_kind, scheme_for};
pub use listener::{Listener, Transport, shorten_path};
pub use project::{GroupSource, Repo, Scope};
pub use rules::{Rules, glob_match, process_match};
pub use view::{GroupBy, Query, SortBy};

use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Server {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub command: String,
    pub cmdline: String,
    pub exe: Option<PathBuf>,
    pub user: String,
    pub cwd: Option<PathBuf>,
    pub listeners: Vec<Listener>,
    pub repo: Option<Repo>,
    pub kind: Kind,
    /// What the signature table decided this is — "PostgreSQL", "Next.js" — as
    /// distinct from the process that happens to be running it.
    pub service: Option<String>,
    /// The path that means healthy, from the signature.
    pub health_path: Option<String>,
    /// What a user would paste into a client, from the signature.
    pub uri: Option<String>,
    /// Anything the signature wanted to say about this service.
    pub note: Option<String>,
    /// A named handshake from the signature, for a protocol that will not speak
    /// until spoken to.
    pub handshake: Option<String>,
    /// What the service said on connect, if anything.
    pub banner: Option<Vec<u8>>,
    /// Why it was identified as it was, in the words the signature table used.
    pub evidence: Vec<String>,
    /// True when a handshake for this service's protocol was run and the reply
    /// was not the one that protocol gives. The name came from the port, and
    /// the socket disagrees with it.
    pub unconfirmed: bool,
    /// The version the service volunteered, where its protocol offers one.
    pub version: Option<String>,
    /// What a gRPC server said about its own health. `None` covers both "not
    /// asked" and "asked, and it does not implement the health service" —
    /// most do not, and not answering is running rather than broken.
    pub serving: Option<bool>,
    /// Where the world can reach this, when something is exposing it.
    pub exposed: Option<crate::tunnel::Exposure>,
    /// What it presented at the TLS handshake, where it speaks TLS.
    pub certificate: Option<crate::certificate::Certificate>,
    /// The command that started this and at least one other service.
    pub launcher: Option<Launcher>,
    /// The container behind this port, where the runtime published one.
    pub container: Option<crate::docker::Container>,
    pub started_at: u64,
    /// When quarry first saw this listening, if it appeared while quarry was
    /// watching. `None` for anything that was already there — the first scan
    /// is not news, and marking the whole machine as new would say nothing.
    pub appeared: Option<Instant>,
    pub cpu: f32,
    pub mem: u64,
    pub health: Health,
}

impl Server {
    /// The listener we treat as the service's front door. Sorted so that a TCP
    /// port comes before a unix socket: a service reachable both ways is
    /// usually reached over the port.
    pub fn primary(&self) -> Option<&Listener> {
        self.listeners.first()
    }

    /// Who to talk to in order to stop or restart this.
    ///
    /// The container where there is one. A published port is held by the
    /// runtime's forwarder, not by the container, so the pid quarry can see is
    /// the wrong thing to signal — at best it breaks the forward and leaves
    /// the container running, at worst it is part of the daemon.
    pub fn lifecycle(&self) -> crate::lifecycle::Target {
        match &self.container {
            Some(c) => crate::lifecycle::Target::Container(Box::new(c.clone())),
            None => crate::lifecycle::Target::Process {
                pid: self.pid,
                port: self.listeners.iter().find(|l| !l.is_unix()).map(|l| l.port),
            },
        }
    }

    pub fn primary_port(&self) -> u16 {
        self.listeners.first().map(|l| l.port).unwrap_or(0)
    }

    /// What to show where a port would go.
    pub fn primary_label(&self) -> String {
        self.primary().map(|l| l.label()).unwrap_or_default()
    }

    /// The same, for a column of fixed width.
    pub fn primary_column(&self) -> String {
        self.primary().map(|l| l.column()).unwrap_or_default()
    }

    /// A TLS certificate that has expired, is about to, or is for a name other
    /// than the one being used. Each of these is otherwise diagnosed by reading
    /// a browser error.
    pub fn certificate_trouble(&self, now: u64) -> Option<String> {
        let c = self.certificate.as_ref()?;
        let now = now as i64;
        if c.expired(now) {
            return Some("certificate expired".to_string());
        }
        if c.expires_within(7 * 86_400, now) {
            return Some("certificate expires within a week".to_string());
        }
        // quarry reaches every service as `localhost`, so that is the name a
        // client checks the certificate against.
        let host = "localhost";
        if !c.names.is_empty() && !c.covers(host) {
            return Some(format!("certificate is not for {host}"));
        }
        None
    }

    /// Whether a probe result belongs to this service.
    ///
    /// Pid *and* port, always. A pid can be reused between scans, and a
    /// container runtime publishes every port from one process — so several
    /// services share a pid, and matching on it alone gives them all the
    /// health of whichever was probed first. Written twice, it was right in
    /// one place and wrong in the other.
    pub fn answers(&self, pid: u32, port: u16) -> bool {
        self.pid == pid && self.listeners.iter().any(|l| l.port == port)
    }

    /// Whether handing this to a browser is a promise that can be kept.
    ///
    /// Two conditions, and they were being written out together wherever the
    /// question came up: the kind has to be something a browser speaks, and
    /// there has to be a port — no browser opens a unix socket.
    pub fn opens_in_a_browser(&self) -> bool {
        self.kind.opens_in_a_browser() && !self.is_socket_only()
    }

    /// True when nothing here can be reached over a port.
    pub fn is_socket_only(&self) -> bool {
        self.listeners.iter().all(|l| l.is_unix())
    }

    pub fn scheme(&self) -> &'static str {
        if let Health::Http { scheme, .. } = &self.health {
            return scheme;
        }
        scheme_for(self.primary_port())
    }

    pub fn url(&self) -> String {
        if let Some(uri) = &self.uri {
            return uri.clone();
        }
        match self.primary() {
            Some(l) if l.is_unix() => format!(
                "unix:{}",
                l.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
            _ => format!("{}://localhost:{}", self.scheme(), self.primary_port()),
        }
    }

    /// What to call this service in a list: the name the signature gave it, or
    /// failing that the process, tidied.
    pub fn service_name(&self) -> String {
        // The container first: the daemon knows what is behind the port, and a
        // signature can only guess from the host process that published it —
        // which is the runtime, and says nothing about the service.
        if let Some(container) = &self.container {
            return container.display_name().to_string();
        }
        if let Some(name) = &self.service {
            return name.clone();
        }
        friendly_process(&self.command, &self.cmdline)
    }

    /// The label a row leads with — the repo name if we found one, else the
    /// process, trimmed of the paths that make `node` rows unreadable.
    pub fn title(&self) -> String {
        if let Some(repo) = &self.repo {
            return repo.name.clone();
        }
        if let Some(project) = self.container.as_ref().and_then(|c| c.project.as_deref()) {
            return project.to_string();
        }
        if let Some(folder) = self.folder_name() {
            return folder;
        }
        friendly_process(&self.command, &self.cmdline)
    }

    /// The project a service belongs to: its repository if it has one, else the
    /// directory it is running in. A process started in a plain folder is still
    /// a project to whoever started it.
    pub fn group_key(&self) -> String {
        if let Some(r) = &self.repo {
            return r.name.clone();
        }
        // A Compose project is a project, whether or not its directory
        // resolved to a repository.
        if let Some(project) = self.container.as_ref().and_then(|c| c.project.as_deref()) {
            return project.to_string();
        }
        if let Some(folder) = self.folder_name() {
            return folder;
        }
        if self.kind == Kind::System {
            "system".into()
        } else {
            "unattributed".into()
        }
    }

    /// Whether this belongs to the project in scope.
    ///
    /// By the repository it resolved to, which is why a container counts: a
    /// Compose stack is attributed to the directory its file lives in, so a
    /// database declared in the repository is part of the project as much as
    /// the server started from a shell in it.
    pub fn in_scope(&self, scope: &Scope) -> bool {
        self.repo
            .as_ref()
            .is_some_and(|r| r.main_root == scope.root)
    }

    /// Which checkout of one project this came from.
    ///
    /// Inside a single repository, what distinguishes two services is not the
    /// project — they share it — but the branch. That is also what a person
    /// calls a worktree: not `.worktrees/quarry/feat/repo-scope` but
    /// `feat/repo-scope`. A detached head has no branch to call it, so the
    /// directory does.
    pub fn worktree_key(&self) -> String {
        let Some(repo) = &self.repo else {
            return self.group_key();
        };
        if let Some(branch) = &repo.branch {
            return branch.clone();
        }
        repo.root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| repo.root.display().to_string())
    }

    pub fn group_source(&self) -> GroupSource {
        if self.repo.is_some() {
            GroupSource::Repo
        } else if self.container.as_ref().is_some_and(|c| c.project.is_some())
            || self.folder_name().is_some()
        {
            // A Compose project and a directory are the same kind of claim: a
            // name that came from somewhere real, but not from a repository.
            GroupSource::Folder
        } else if self.kind == Kind::System {
            GroupSource::System
        } else {
            GroupSource::Unattributed
        }
    }

    /// The working directory's name, when it is specific enough to mean
    /// something. `/`, `$HOME` and the temp directory name nothing in
    /// particular, so they are not worth grouping by.
    pub fn folder_name(&self) -> Option<String> {
        let cwd = self.cwd.as_ref()?;
        if is_anonymous_dir(cwd) {
            return None;
        }
        let name = cwd.file_name()?.to_string_lossy().to_string();
        (!name.is_empty()).then_some(name)
    }

    pub fn uptime(&self, now: u64) -> Option<Duration> {
        now.checked_sub(self.started_at).map(Duration::from_secs)
    }

    /// Whether a filter matches. `needle` must already be lowercase — the
    /// caller lowercases once rather than every service lowercasing it again.
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        // Searching by project name has to work whether that name came from a
        // repository or from a directory. Checked in rough order of how often
        // it is what the user meant, and how cheap it is to look at.
        if self.listeners.iter().any(|l| port_contains(l.port, needle)) {
            return true;
        }
        if contains_ci(self.kind.label(), needle) || contains_ci(&self.command, needle) {
            return true;
        }
        if let Some(service) = &self.service
            && contains_ci(service, needle)
        {
            return true;
        }
        if let Some(repo) = &self.repo {
            if contains_ci(&repo.name, needle) {
                return true;
            }
        } else if let Some(folder) = self.folder_name()
            && contains_ci(&folder, needle)
        {
            return true;
        }
        contains_ci(&self.cmdline, needle)
    }
}

/// Whether a port's decimal form contains `needle`, without rendering it.
/// Five digits fit in a stack buffer; a filter runs this once per listener per
/// keystroke.
fn port_contains(port: u16, needle: &str) -> bool {
    let mut buf = [0u8; 5];
    let mut n = port;
    let mut len = 0;
    loop {
        buf[4 - len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
        if n == 0 {
            break;
        }
    }
    let digits = &buf[5 - len..];
    // SAFETY-free: the bytes written above are ASCII digits by construction.
    let text = std::str::from_utf8(digits).unwrap_or("");
    text.contains(needle)
}

/// `haystack.to_lowercase().contains(needle)` without the allocation.
///
/// Filtering runs over every service on every keystroke, and allocating two
/// lowercase copies per service per character is most of what that used to
/// cost. ASCII takes the fast path, which is every port number and nearly every
/// process name; anything else falls back to the correct-but-slower form.
pub fn contains_ci(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if haystack.len() < needle_lower.len() {
        return false;
    }
    if haystack.is_ascii() && needle_lower.is_ascii() {
        let hay = haystack.as_bytes();
        let needle = needle_lower.as_bytes();
        return hay.windows(needle.len()).any(|w| {
            w.iter()
                .zip(needle)
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
        });
    }
    haystack.to_lowercase().contains(needle_lower)
}

/// Directories that name no particular project.
fn is_anonymous_dir(path: &std::path::Path) -> bool {
    if path.parent().is_none() {
        return true; // the filesystem root
    }
    let anonymous = ["/", "/tmp", "/var", "/usr", "/private/tmp", "/var/root"];
    if anonymous.iter().any(|a| path == std::path::Path::new(a)) {
        return true;
    }
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => path == std::path::Path::new(&home),
        _ => false,
    }
}

/// A command that started several of the services on screen.
///
/// Not tidiness: knowing that nine things came from one command tells you they
/// stop together, and which single process to stop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launcher {
    pub pid: u32,
    pub command: String,
}
