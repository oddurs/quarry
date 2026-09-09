//! The supervised background loop.
//!
//! Scanning and probing happen on one thread that outlives individual scans.
//! The UI never blocks on it: it receives messages, and if the thread is slow,
//! wedged or dead, the UI keeps drawing and says so.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::diag;
use crate::engine::Engine;
use crate::model::{Health, Server};
use crate::probe::{Pool, Prober, Target};

/// How long between automatic rescans.
pub const AUTO_REFRESH: Duration = Duration::from_secs(6);
/// A scan slower than this is worth complaining about.
pub const SLOW_SCAN: Duration = Duration::from_secs(2);
/// Bounded so a stalled UI cannot make the scanner grow memory without limit.
pub const QUEUE_DEPTH: usize = 512;

#[derive(Debug)]
pub enum Msg {
    Scanning,
    Servers(Vec<Server>),
    Health {
        pid: u32,
        port: u16,
        health: Health,
        /// What the service said on connect, if anything — evidence for a
        /// second pass at identification.
        banner: Option<Vec<u8>>,
    },
    /// The scan itself failed. Carries whether a retry is worth anything.
    ScanFailed {
        detail: String,
        transient: bool,
    },
    Warning(String),
}

enum Command {
    Refresh,
    Shutdown,
}

pub struct Settings {
    pub auto_refresh: Duration,
    pub probe_workers: usize,
    /// Per-port health paths, from the user's config.
    pub health: std::collections::BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_refresh: AUTO_REFRESH,
            probe_workers: crate::probe::WORKERS,
            health: Default::default(),
        }
    }
}

impl Settings {
    pub fn from_config(config: &Config) -> Self {
        Self {
            auto_refresh: config.refresh(),
            probe_workers: config.probe_workers,
            health: config.health.clone(),
        }
    }

    fn health_path(&self, port: u16) -> String {
        self.health
            .get(&port.to_string())
            .or_else(|| self.health.get("*"))
            .cloned()
            .unwrap_or_else(|| "/".to_string())
    }
}

/// The UI's handle on the background thread.
pub struct Handle {
    commands: Sender<Command>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Handle {
    /// Ask for an immediate rescan. Silently ignored if the thread has died —
    /// the caller finds out from [`Handle::is_alive`], not from here.
    pub fn refresh(&self) {
        let _ = self.commands.send(Command::Refresh);
    }

    pub fn is_alive(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stop the loop and wait for it. Called on quit, and on drop.
    pub fn shutdown(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start the loop. Returns the handle and the receiver the UI drains.
pub fn spawn(engine: Engine, prober: Arc<dyn Prober>, config: Settings) -> (Handle, Receiver<Msg>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (msg_tx, msg_rx) = mpsc::sync_channel::<Msg>(QUEUE_DEPTH);

    let thread = std::thread::Builder::new()
        .name("quarry-scanner".into())
        .spawn(move || run(engine, prober, config, cmd_rx, msg_tx))
        .expect("spawn scanner thread");

    (
        Handle {
            commands: cmd_tx,
            thread: Some(thread),
        },
        msg_rx,
    )
}

/// Send without ever blocking the scanner on a busy UI. A dropped update is
/// recoverable — the next scan sends it again — but a blocked scanner is not.
fn emit(tx: &SyncSender<Msg>, msg: Msg) -> bool {
    match tx.try_send(msg) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            diag::warn("runtime", "message queue full; dropped an update");
            true
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

fn run(
    mut engine: Engine,
    prober: Arc<dyn Prober>,
    config: Settings,
    commands: Receiver<Command>,
    out: SyncSender<Msg>,
) {
    let pool = Pool::with_workers(prober, config.probe_workers);
    let mut consecutive_failures = 0u32;

    loop {
        if !emit(&out, Msg::Scanning) {
            return;
        }

        let started = Instant::now();
        let report = engine.scan();
        let elapsed = started.elapsed();
        if elapsed > SLOW_SCAN {
            diag::warn("runtime", format!("scan took {}ms", elapsed.as_millis()));
        }

        match report {
            Ok(report) => {
                consecutive_failures = 0;
                for warning in report.warnings {
                    if !emit(&out, Msg::Warning(warning)) {
                        return;
                    }
                }
                for s in &report.servers {
                    if let Some(l) = s.listeners.first() {
                        let mut target = Target::from_listener(s.pid, l, s.kind);
                        target.path = s
                            .health_path
                            .clone()
                            .unwrap_or_else(|| config.health_path(l.port));
                        target.handshake = s.handshake.clone();
                        pool.submit(target);
                    }
                }
                if !emit(&out, Msg::Servers(report.servers)) {
                    return;
                }
            }
            Err(e) => {
                consecutive_failures += 1;
                diag::error("runtime", format!("scan failed: {e}"));
                let transient = e.transient;
                if !emit(
                    &out,
                    Msg::ScanFailed {
                        detail: e.to_string(),
                        transient,
                    },
                ) {
                    return;
                }
            }
        }

        // Back off after repeated failures rather than hammering a broken
        // `lsof` every six seconds forever.
        let wait = if consecutive_failures > 0 {
            let factor = 2u32.saturating_pow(consecutive_failures.min(4));
            config
                .auto_refresh
                .saturating_mul(factor)
                .min(Duration::from_secs(60))
        } else {
            config.auto_refresh
        };

        // Stream probe results until it is time to scan again.
        let deadline = Instant::now() + wait;
        loop {
            for o in pool.drain() {
                if !emit(
                    &out,
                    Msg::Health {
                        pid: o.pid,
                        port: o.port,
                        health: o.health,
                        banner: o.banner,
                    },
                ) {
                    return;
                }
            }
            match commands.recv_timeout(Duration::from_millis(80)) {
                Ok(Command::Refresh) => break,
                Ok(Command::Shutdown) => return,
                Err(RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::probe::ScriptedProber;
    use crate::source::{StaticCwds, StaticProcesses, StaticSockets};
    use crate::testkit;

    fn fixture_engine(sockets: Vec<crate::source::RawSocket>) -> Engine {
        Engine::new(
            Box::new(StaticSockets::new(sockets)),
            Box::new(StaticProcesses::default()),
            Box::new(StaticCwds::default()),
        )
    }

    fn drain_until<F>(rx: &Receiver<Msg>, mut done: F) -> Vec<Msg>
    where
        F: FnMut(&[Msg]) -> bool,
    {
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left.min(Duration::from_millis(200))) {
                Ok(m) => got.push(m),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if done(&got) {
                break;
            }
        }
        got
    }

    #[test]
    fn a_scan_produces_servers_then_health() {
        let engine = fixture_engine(vec![testkit::socket(1, "node", 3000)]);
        let prober = ScriptedProber::new(testkit::scenario_health());
        let (mut handle, rx) = spawn(
            engine,
            Arc::new(prober),
            Settings {
                auto_refresh: Duration::from_secs(30),
                probe_workers: 2,
                ..Default::default()
            },
        );

        let msgs = drain_until(&rx, |m| m.iter().any(|m| matches!(m, Msg::Health { .. })));
        assert!(msgs.iter().any(|m| matches!(m, Msg::Scanning)));
        assert!(
            msgs.iter()
                .any(|m| matches!(m, Msg::Servers(s) if s.len() == 1)),
            "expected one server, got {msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| matches!(m, Msg::Health { port: 3000, .. }))
        );
        handle.shutdown();
        assert!(!handle.is_alive());
    }

    #[test]
    fn a_failing_source_reports_and_keeps_running() {
        let engine = Engine::new(
            Box::new(StaticSockets::failing("lsof: not found")),
            Box::new(StaticProcesses::default()),
            Box::new(StaticCwds::default()),
        );
        let (mut handle, rx) = spawn(
            engine,
            Arc::new(ScriptedProber::new(Default::default())),
            Settings {
                auto_refresh: Duration::from_millis(100),
                probe_workers: 1,
                ..Default::default()
            },
        );
        let msgs = drain_until(&rx, |m| {
            m.iter().any(|m| matches!(m, Msg::ScanFailed { .. }))
        });
        assert!(
            msgs.iter().any(|m| matches!(m, Msg::ScanFailed { .. })),
            "failure was not reported"
        );
        assert!(handle.is_alive(), "the loop must survive a failed scan");
        handle.shutdown();
    }

    #[test]
    fn shutdown_is_prompt_even_mid_wait() {
        let engine = fixture_engine(vec![testkit::socket(1, "node", 3000)]);
        let (mut handle, _rx) = spawn(
            engine,
            Arc::new(ScriptedProber::new(Default::default())),
            Settings {
                auto_refresh: Duration::from_secs(300),
                probe_workers: 1,
                ..Default::default()
            },
        );
        std::thread::sleep(Duration::from_millis(150));
        let started = Instant::now();
        handle.shutdown();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "shutdown waited on the refresh timer"
        );
    }

    #[test]
    fn dropping_the_handle_stops_the_thread() {
        let engine = fixture_engine(vec![testkit::socket(1, "node", 3000)]);
        let (handle, _rx) = spawn(
            engine,
            Arc::new(ScriptedProber::new(Default::default())),
            Settings {
                auto_refresh: Duration::from_secs(300),
                probe_workers: 1,
                ..Default::default()
            },
        );
        drop(handle); // Hangs here if shutdown-on-drop regresses.
    }

    #[test]
    fn a_dropped_receiver_ends_the_loop() {
        let engine = fixture_engine(vec![testkit::socket(1, "node", 3000)]);
        let (handle, rx) = spawn(
            engine,
            Arc::new(ScriptedProber::new(Default::default())),
            Settings {
                auto_refresh: Duration::from_millis(50),
                probe_workers: 1,
                ..Default::default()
            },
        );
        drop(rx);
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.is_alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!handle.is_alive(), "the loop outlived its receiver");
    }
}
