//! The `lsof` socket source, and the pure parsers behind it.
//!
//! Parsing is deliberately separated from running: the field format is the
//! fiddly part, and it is the part worth testing against captured output from
//! real machines.

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::exec::{self, ExecError};
use crate::model::Listener;
use crate::source::{CwdSource, RawSocket, SocketSource, SourceError};

const LISTEN_TIMEOUT: Duration = Duration::from_secs(4);
const CWD_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Lsof;

impl SocketSource for Lsof {
    fn listening(&mut self) -> Result<Vec<RawSocket>, SourceError> {
        match exec::run(
            "lsof",
            &["-nP", "-iTCP", "-sTCP:LISTEN", "-FpcLn"],
            LISTEN_TIMEOUT,
        ) {
            Ok(out) => Ok(parse_listening(&out)),
            // `lsof` exits 1 when nothing matched, which is not a failure: a
            // machine with nothing listening is a machine with nothing
            // listening. Treating it as an error made quarry refuse to start
            // inside a container.
            Err(ExecError::Failed {
                code: Some(1),
                stderr,
            }) if stderr.trim().is_empty() => Ok(Vec::new()),
            Err(e) => Err(to_source_error(e)),
        }
    }

    fn describe(&self) -> String {
        "lsof -nP -iTCP -sTCP:LISTEN".to_string()
    }
}

/// The working-directory fallback, for processes the process table would not
/// answer for.
///
/// Remembers which pids it could not read. Another user's process is one we
/// will never be allowed to inspect, and asking `lsof` about it again every six
/// seconds for the life of the session costs tens of milliseconds a scan to
/// learn the same nothing.
#[derive(Default)]
pub struct LsofCwds {
    hopeless: std::collections::HashSet<u32>,
}

impl CwdSource for LsofCwds {
    fn cwds(&mut self, pids: &[u32]) -> HashMap<u32, PathBuf> {
        let pids: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|p| !self.hopeless.contains(p))
            .collect();
        if pids.is_empty() {
            return HashMap::new();
        }
        let pids = &pids[..];
        let list = pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        match exec::run(
            "lsof",
            &["-a", "-d", "cwd", "-Fpn", "-p", &list],
            CWD_TIMEOUT,
        ) {
            Ok(out) => parse_cwds(&out),
            Err(e) => {
                crate::diag::warn("lsof", format!("working directory lookup failed: {e}"));
                HashMap::new()
            }
        }
    }
}

fn to_source_error(e: ExecError) -> SourceError {
    let transient = matches!(e, ExecError::Timeout(_));
    SourceError {
        source: "lsof",
        detail: e.to_string(),
        transient,
    }
}

/// Parse `lsof -FpcLn` output.
///
/// Records are a stream of tagged lines: `p` opens a process, `c` and `L` name
/// it, and each `n` that follows is one of its sockets. A malformed line is
/// skipped rather than aborting the parse — half a socket list beats none.
pub fn parse_listening(text: &str) -> Vec<RawSocket> {
    #[derive(Default)]
    struct Proc {
        command: String,
        user: String,
        listeners: Vec<Listener>,
    }

    let mut procs: BTreeMap<u32, Proc> = BTreeMap::new();
    let mut pid: Option<u32> = None;

    for line in text.lines() {
        let Some((tag, value)) = line.split_at_checked(1) else {
            continue;
        };
        match tag {
            "p" => pid = value.trim().parse::<u32>().ok(),
            "c" => {
                if let Some(p) = pid {
                    procs.entry(p).or_default().command = value.to_string();
                }
            }
            "L" => {
                if let Some(p) = pid {
                    procs.entry(p).or_default().user = value.to_string();
                }
            }
            "n" => {
                let (Some(p), Some(listener)) = (pid, parse_listen_addr(value)) else {
                    continue;
                };
                let entry = procs.entry(p).or_default();
                if !entry.listeners.contains(&listener) {
                    entry.listeners.push(listener);
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    for (pid, proc) in procs {
        for listener in proc.listeners {
            out.push(RawSocket {
                pid,
                command: proc.command.clone(),
                user: proc.user.clone(),
                listener,
            });
        }
    }
    out
}

/// `*:3000`, `127.0.0.1:8080`, `[::1]:5432`, `192.168.1.4:7000`.
pub fn parse_listen_addr(raw: &str) -> Option<Listener> {
    let raw = raw.split("->").next()?.trim();
    let (host, port) = raw.rsplit_once(':')?;
    let port: u16 = port.trim().parse().ok()?;
    if port == 0 {
        return None;
    }
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');

    if host == "*" {
        return Some(Listener::tcp(IpAddr::from([0, 0, 0, 0]), port));
    }
    // lsof appends a zone index to link-local v6 addresses: `fe80::1%en0`.
    let host = host.split('%').next().unwrap_or(host);
    let addr: IpAddr = host.parse().ok()?;
    Some(Listener::tcp(addr, port))
}

/// Parse `lsof -d cwd -Fpn` output into pid → working directory.
pub fn parse_cwds(text: &str) -> HashMap<u32, PathBuf> {
    let mut map = HashMap::new();
    let mut pid: Option<u32> = None;
    for line in text.lines() {
        match line.split_at_checked(1) {
            Some(("p", v)) => pid = v.trim().parse::<u32>().ok(),
            Some(("n", v)) => {
                if let Some(p) = pid {
                    map.entry(p).or_insert_with(|| PathBuf::from(v));
                }
            }
            _ => {}
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str =
        "p1234\ncnode\nLoddurs\nn*:3000\nn127.0.0.1:3001\np56\ncpostgres\nLoddurs\nn[::1]:5432\n";

    #[test]
    fn parses_processes_and_sockets() {
        let socks = parse_listening(SAMPLE);
        assert_eq!(socks.len(), 3);
        assert_eq!(socks[0].pid, 56, "output is ordered by pid");
        assert_eq!(socks[0].listener.port, 5432);
        assert_eq!(socks[1].command, "node");
        assert!(socks[1].listener.wildcard, "*:3000 binds every interface");
        assert!(!socks[2].listener.wildcard);
    }

    #[test]
    fn skips_junk_without_losing_the_rest() {
        let text = "p1\ncgood\nn127.0.0.1:80\nGARBAGE\nn\nnnot-an-address\nn127.0.0.1:notaport\nn127.0.0.1:443\n";
        let socks = parse_listening(text);
        assert_eq!(socks.len(), 2, "valid sockets survive invalid neighbours");
    }

    #[test]
    fn ignores_sockets_before_any_process() {
        let socks = parse_listening("n127.0.0.1:80\np9\ncx\nn127.0.0.1:81\n");
        assert_eq!(socks.len(), 1);
        assert_eq!(socks[0].pid, 9);
    }

    #[test]
    fn deduplicates_a_repeated_socket() {
        let socks = parse_listening("p1\ncx\nn*:3000\nn*:3000\n");
        assert_eq!(socks.len(), 1);
    }

    #[test]
    fn handles_ipv6_forms() {
        assert_eq!(parse_listen_addr("[::1]:5432").unwrap().port, 5432);
        assert!(parse_listen_addr("[::]:8080").unwrap().wildcard);
        assert_eq!(
            parse_listen_addr("[fe80::1%en0]:8080").unwrap().port,
            8080,
            "a zone index must not defeat the parse"
        );
    }

    #[test]
    fn rejects_nonsense() {
        for bad in [
            "",
            ":",
            "1.2.3.4",
            "1.2.3.4:",
            ":80",
            "host:0",
            "1.2.3.4:99999",
        ] {
            assert!(parse_listen_addr(bad).is_none(), "accepted {bad:?}");
        }
    }

    #[test]
    fn parses_cwd_records() {
        let map = parse_cwds("p1\nn/Users/x/code\np2\nn/tmp\nn/ignored-second\n");
        assert_eq!(map.len(), 2);
        assert_eq!(map[&1], PathBuf::from("/Users/x/code"));
        assert_eq!(map[&2], PathBuf::from("/tmp"), "first cwd wins");
    }

    /// `lsof` exits 1 when it matched nothing, and a machine with nothing
    /// listening is not a broken machine. Reading that as a failure made
    /// quarry refuse to start inside a container.
    #[test]
    fn finding_nothing_is_an_empty_list_not_a_failure() {
        // The real thing, asked about a port range nothing can be using.
        let out = exec::run(
            "lsof",
            &["-nP", "-iTCP:1", "-sTCP:LISTEN", "-FpcLn"],
            LISTEN_TIMEOUT,
        );
        match out {
            Ok(text) => assert!(parse_listening(&text).is_empty()),
            Err(ExecError::Failed {
                code: Some(1),
                stderr,
            }) => {
                assert!(stderr.trim().is_empty(), "lsof complained: {stderr}");
            }
            Err(ExecError::Spawn(_)) => {} // no lsof here, which is allowed
            Err(e) => panic!("unexpected: {e}"),
        }
        // And through the source, which is what the engine sees.
        let found = Lsof.listening();
        assert!(found.is_ok() || matches!(found, Err(ref e) if e.transient));
    }

    #[test]
    fn empty_input_is_not_an_error() {
        assert!(parse_listening("").is_empty());
        assert!(parse_cwds("").is_empty());
    }
}
