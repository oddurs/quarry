//! The seams. Everything that reaches outside the process — sockets, process
//! tables, working directories — is behind one of these traits, so the whole
//! discovery pipeline can be driven from fixtures with no machine underneath.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::model::Listener;

/// One listening socket, before we know anything about the repo behind it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSocket {
    pub pid: u32,
    /// The short name the OS reports, which may be truncated.
    pub command: String,
    pub user: String,
    pub listener: Listener,
}

/// What a process table can tell us. Every field is optional in practice, so
/// the default is a usable — if empty — record rather than an error.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcInfo {
    pub cmdline: String,
    pub name: String,
    pub exe: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub ppid: Option<u32>,
    pub started_at: u64,
    pub cpu: f32,
    pub mem: u64,
}

#[derive(Debug)]
pub struct SourceError {
    pub source: &'static str,
    pub detail: String,
    /// True when a retry might work — a timeout rather than a missing binary.
    pub transient: bool,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.source, self.detail)
    }
}

impl std::error::Error for SourceError {}

/// Where listening sockets come from.
pub trait SocketSource: Send {
    fn listening(&mut self) -> Result<Vec<RawSocket>, SourceError>;
    /// Shown by `--doctor`.
    fn describe(&self) -> String;
}

/// Where process metadata comes from.
pub trait ProcessSource: Send {
    fn refresh(&mut self, pids: &[u32]);
    fn info(&self, pid: u32) -> Option<ProcInfo>;
    fn describe(&self) -> String;
}

/// A last-resort lookup for working directories the process table would not
/// give us. Separate from `ProcessSource` because it is a different, and much
/// more expensive, mechanism.
pub trait CwdSource: Send {
    fn cwds(&mut self, pids: &[u32]) -> HashMap<u32, PathBuf>;
}

/// A fixed set of sockets — the backbone of every deterministic test.
pub struct StaticSockets {
    pub sockets: Vec<RawSocket>,
    pub error: Option<SourceError>,
}

impl StaticSockets {
    pub fn new(sockets: Vec<RawSocket>) -> Self {
        Self {
            sockets,
            error: None,
        }
    }

    /// A source that always fails, for exercising degraded paths.
    pub fn failing(detail: &str) -> Self {
        Self {
            sockets: Vec::new(),
            error: Some(SourceError {
                source: "fixture",
                detail: detail.to_string(),
                transient: true,
            }),
        }
    }
}

impl SocketSource for StaticSockets {
    fn listening(&mut self) -> Result<Vec<RawSocket>, SourceError> {
        match self.error.take() {
            Some(e) => Err(e),
            None => Ok(self.sockets.clone()),
        }
    }

    fn describe(&self) -> String {
        format!("fixture ({} sockets)", self.sockets.len())
    }
}

#[derive(Default)]
pub struct StaticProcesses {
    pub procs: HashMap<u32, ProcInfo>,
}

impl StaticProcesses {
    pub fn new(procs: HashMap<u32, ProcInfo>) -> Self {
        Self { procs }
    }
}

impl ProcessSource for StaticProcesses {
    fn refresh(&mut self, _pids: &[u32]) {}

    fn info(&self, pid: u32) -> Option<ProcInfo> {
        self.procs.get(&pid).cloned()
    }

    fn describe(&self) -> String {
        format!("fixture ({} processes)", self.procs.len())
    }
}

#[derive(Default)]
pub struct StaticCwds {
    pub cwds: HashMap<u32, PathBuf>,
}

impl CwdSource for StaticCwds {
    fn cwds(&mut self, pids: &[u32]) -> HashMap<u32, PathBuf> {
        pids.iter()
            .filter_map(|p| self.cwds.get(p).map(|c| (*p, c.clone())))
            .collect()
    }
}

/// A `CwdSource` that never has an answer — the common case once the process
/// table has already succeeded.
pub struct NoCwds;

impl CwdSource for NoCwds {
    fn cwds(&mut self, _pids: &[u32]) -> HashMap<u32, PathBuf> {
        HashMap::new()
    }
}
