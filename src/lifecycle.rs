//! Stopping and restarting a service.
//!
//! Two things make this more than `kill(2)`.
//!
//! The first is that the process holding a port is not always the thing that
//! owns the service. A container's published port belongs to the runtime's
//! forwarder — `docker-proxy`, or OrbStack's helper — and signalling that does
//! not stop the container. On a bad day it takes the daemon down and every
//! other container with it. So an instruction goes to whatever actually owns
//! the lifetime: the daemon for a container, the kernel for a process.
//!
//! The second is that a restart is not a stop followed by a start. Most dev
//! servers are already supervised — by `npm run dev`, by `nodemon`, by a
//! Compose restart policy — and something else will bring them back. Starting
//! a second copy on a port that is about to be reclaimed produces two
//! processes fighting over one socket. So after the process exits we watch the
//! port for a moment, and only start it ourselves if nothing else did.
//!
//! Everything here blocks for seconds at a time and none of it runs on the UI
//! thread. [`perform`] is called from a worker and reports back as a message.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

use crate::docker::Container;

/// How long to wait for a process to act on SIGTERM before giving up on it.
/// Escalating to SIGKILL is not ours to decide: force-kill is a separate
/// command with its own confirmation, and a restart must not smuggle one in.
const EXIT_GRACE: Duration = Duration::from_secs(8);

/// How long to watch a freed port before concluding that nothing else is going
/// to claim it. `nodemon` and `npm run dev` are back well inside a second;
/// two is slack for a slower supervisor without leaving the service down for
/// noticeably longer than a restart takes anyway.
const RECLAIM_WINDOW: Duration = Duration::from_secs(2);

const POLL: Duration = Duration::from_millis(100);

/// What owns a service's lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A container. The daemon starts and stops it; we ask the daemon.
    Container(Box<Container>),
    /// A plain process. We signal it, and we start it again ourselves if
    /// nothing else does.
    Process {
        pid: u32,
        /// The port to watch after it exits. `None` for a unix socket, where
        /// there is nothing to watch.
        port: Option<u16>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Stop,
    Restart,
    /// SIGKILL, or the daemon's equivalent. No grace period, no clean shutdown.
    Kill,
}

impl Op {
    /// Also the container API's endpoint name, which is why these are the
    /// words rather than a synonym.
    pub fn verb(self) -> &'static str {
        match self {
            Op::Stop => "stop",
            Op::Restart => "restart",
            Op::Kill => "kill",
        }
    }

    /// For a toast raised after the fact.
    pub fn past(self) -> &'static str {
        match self {
            Op::Stop => "stopped",
            Op::Restart => "restarted",
            Op::Kill => "killed",
        }
    }

    /// For a toast raised while it is still happening. Spelled out rather than
    /// built from [`Op::verb`], which would produce "stoping".
    pub fn in_progress(self) -> &'static str {
        match self {
            Op::Stop => "stopping",
            Op::Restart => "restarting",
            Op::Kill => "killing",
        }
    }
}

/// Carry out an instruction. Returns what to tell the user either way — the
/// failures here are all things a person can act on, so none of them are worth
/// reducing to "failed".
pub fn perform(target: &Target, op: Op) -> Result<String, String> {
    match target {
        Target::Container(c) => container(c, op),
        Target::Process { pid, port } => process(*pid, *port, op),
    }
}

fn container(c: &Container, op: Op) -> Result<String, String> {
    crate::docker::act(c, op.verb())?;
    let name = c.display_name();
    Ok(match op {
        Op::Stop => format!("stopped container {name}"),
        Op::Restart => format!("restarted container {name}"),
        Op::Kill => format!("killed container {name}"),
    })
}

fn process(pid: u32, port: Option<u16>, op: Op) -> Result<String, String> {
    match op {
        Op::Stop => {
            signal(pid, "TERM")?;
            Ok(format!("sent SIGTERM to {pid}"))
        }
        Op::Kill => {
            signal(pid, "KILL")?;
            Ok(format!("sent SIGKILL to {pid}"))
        }
        Op::Restart => restart(pid, port),
    }
}

/// Everything needed to run a process again: its arguments, where it ran, and
/// the environment it ran in.
///
/// Read before anything is signalled, and a restart is abandoned if any of it
/// is missing. Stopping a service we cannot start again is not a restart, and
/// starting it with a different environment than it had is a subtler way to
/// break it than leaving it alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

impl Plan {
    /// What the user is agreeing to, in one line.
    pub fn describe(&self) -> String {
        format!("{} in {}", self.argv.join(" "), self.cwd.display())
    }
}

pub fn plan_for(pid: u32) -> Result<Plan, String> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
            .with_environ(UpdateKind::Always),
    );
    let p = sys
        .process(Pid::from_u32(pid))
        .ok_or_else(|| format!("pid {pid} is already gone"))?;

    let argv: Vec<String> = p
        .cmd()
        .iter()
        .map(|s| s.to_string_lossy().to_string())
        .collect();
    if argv.first().is_none_or(|a| a.is_empty()) {
        return Err(format!("pid {pid} has no command line to run again"));
    }
    let cwd = p
        .cwd()
        .ok_or("cannot see which directory it ran in")?
        .to_path_buf();

    // An empty environment means the kernel would not show us one, not that
    // the process had none — on macOS that happens for anything owned by
    // another user. Substituting quarry's own environment would start the
    // service as a different service.
    let env: Vec<(String, String)> = p
        .environ()
        .iter()
        .filter_map(|e| e.to_str())
        .filter_map(|e| {
            e.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();
    if env.is_empty() {
        return Err("cannot read its environment — restarting would change it".into());
    }

    Ok(Plan { argv, cwd, env })
}

fn restart(pid: u32, port: Option<u16>) -> Result<String, String> {
    // Read the plan first. If it cannot be read, nothing has been signalled
    // and the service is still up.
    let plan = plan_for(pid).map_err(|e| format!("cannot restart pid {pid}: {e}"))?;

    signal(pid, "TERM")?;
    if !wait_for_exit(pid, EXIT_GRACE) {
        return Err(format!(
            "pid {pid} did not exit within {}s of SIGTERM — left it running",
            EXIT_GRACE.as_secs()
        ));
    }

    if let Some(port) = port
        && wait_for_listener(port, RECLAIM_WINDOW)
    {
        return Ok(format!(
            "stopped {pid} — :{port} came back on its own, so quarry left it alone"
        ));
    }

    let (child, log) = spawn(&plan)?;
    Ok(format!(
        "restarted as pid {child} — output in {}",
        log.display()
    ))
}

/// SIGTERM and SIGKILL via `kill(1)` rather than `libc::kill`. The crate denies
/// unsafe code, and this is not the place to make an exception: it runs once,
/// on a worker, when a person has already pressed a key twice.
fn signal(pid: u32, name: &str) -> Result<(), String> {
    // pid 1 is init or launchd. Nothing that listens on a port is pid 1, so
    // reaching here means something upstream is confused, and the cost of
    // being wrong is the machine.
    if pid <= 1 {
        return Err(format!("refusing to signal pid {pid}"));
    }
    let out = Command::new("kill")
        .args([&format!("-{name}"), &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run kill: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&out.stderr);
    let why = why.rsplit(':').next().unwrap_or("").trim();
    Err(match why {
        "" => format!("could not signal {pid}"),
        w => format!("could not signal {pid}: {w}"),
    })
}

/// True once the process has exited.
///
/// A zombie counts as exited. It is still in the process table — it stays
/// there until its parent reaps it, and its parent may be in no hurry — but it
/// has released its sockets, which is the part that matters here. Waiting for
/// the entry to disappear would time out on a process that died immediately.
fn wait_for_exit(pid: u32, within: Duration) -> bool {
    let mut sys = System::new();
    let pids = [Pid::from_u32(pid)];
    let deadline = Instant::now() + within;
    loop {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing(),
        );
        match sys.process(pids[0]) {
            None => return true,
            Some(p) if p.status() == ProcessStatus::Zombie => return true,
            Some(_) => {}
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// True if anything accepts a connection on the port before the deadline.
fn wait_for_listener(port: u16, within: Duration) -> bool {
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let deadline = Instant::now() + within;
    loop {
        if TcpStream::connect_timeout(&addr, POLL).is_ok() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// Start it again, detached from quarry.
///
/// Through `nohup` so that closing the terminal quarry runs in does not take
/// the service with it, and with output appended to a file so that a restarted
/// server's logs go somewhere a person can read rather than into the void — or
/// worse, onto the screen, on top of the TUI.
fn spawn(plan: &Plan) -> Result<(u32, PathBuf), String> {
    let log = log_path(plan)?;
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| format!("could not open {}: {e}", log.display()))?;
    let err = out
        .try_clone()
        .map_err(|e| format!("could not open {}: {e}", log.display()))?;

    let child = Command::new("nohup")
        .args(&plan.argv)
        .current_dir(&plan.cwd)
        .env_clear()
        .envs(plan.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", plan.argv[0]))?;

    let pid = child.id();
    // Reap it, or it sits as a zombie for as long as quarry runs. The process
    // itself is unaffected by us letting go of it.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok((pid, log))
}

/// `$XDG_STATE_HOME/quarry`, or `~/.local/state/quarry`. Logs are state, not
/// configuration, so they do not belong next to `config.toml`.
pub fn log_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(base.join("quarry"))
}

fn log_path(plan: &Plan) -> Result<PathBuf, String> {
    let dir = log_dir().ok_or("no home directory to write a log into")?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    // Named after the command rather than the pid: a restarted service gets a
    // new pid every time, and a log you cannot find twice is not a log.
    let name = std::path::Path::new(&plan.argv[0])
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "service".to_string());
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Ok(dir.join(format!("{safe}.log")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};

    /// A port nothing is using. Bound and released, so there is a window in
    /// which someone else could take it; nothing in the suite is binding
    /// arbitrary ports, and the alternative is a hardcoded port that collides
    /// with whatever the developer happens to be running.
    fn free_port() -> u16 {
        TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .expect("bind an ephemeral port")
            .local_addr()
            .expect("its address")
            .port()
    }

    /// `target/<profile>/examples/hold-port`, found from the running test
    /// binary, which cargo puts in `target/<profile>/deps`.
    ///
    /// A plain `cargo test` builds it. A filtered run — `cargo test --lib` —
    /// does not, so build it rather than failing with something that looks
    /// like a broken test but is a missing artifact.
    fn hold_port_binary() -> PathBuf {
        let exe = std::env::current_exe().expect("the test binary's own path");
        let target = exe
            .parent()
            .and_then(|p| p.parent())
            .expect("target/<profile>");
        let path = target.join("examples").join("hold-port");
        if !path.exists() {
            let release = target.file_name().is_some_and(|n| n == "release");
            let mut cargo = Command::new(env!("CARGO"));
            cargo.args(["build", "--example", "hold-port"]);
            if release {
                cargo.arg("--release");
            }
            let status = cargo.status().expect("run cargo");
            assert!(status.success(), "could not build the hold-port example");
        }
        assert!(path.exists(), "{} is still missing", path.display());
        path
    }

    /// Start one, and wait until it is actually listening.
    fn start_listener(port: u16) -> std::process::Child {
        let child = Command::new(hold_port_binary())
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start hold-port");
        assert!(
            wait_for_listener(port, Duration::from_secs(5)),
            "hold-port never bound :{port}"
        );
        child
    }

    #[test]
    fn a_plan_reads_a_real_process_faithfully() {
        let port = free_port();
        let mut child = start_listener(port);

        let plan = plan_for(child.id()).expect("read the plan");
        assert_eq!(
            plan.argv,
            vec![
                hold_port_binary().to_string_lossy().to_string(),
                port.to_string()
            ],
            "argv has to survive as a vector — joining it and splitting on \
             spaces would break every path with a space in it"
        );
        assert_eq!(
            plan.cwd,
            std::env::current_dir().expect("cwd"),
            "a service started in the wrong directory is not the same service"
        );
        assert!(
            plan.env.iter().any(|(k, _)| k == "PATH"),
            "the environment came back empty: {:?}",
            plan.env
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_plan_for_a_dead_process_fails_rather_than_guessing() {
        let port = free_port();
        let mut child = start_listener(port);
        let pid = child.id();
        let _ = child.kill();
        let _ = child.wait();

        let err = plan_for(pid).unwrap_err();
        assert!(err.contains("gone"), "{err}");
    }

    /// The load-bearing half of a restart: it comes back, in the right place,
    /// with its output somewhere a person can read it.
    #[test]
    fn a_stopped_service_starts_again_on_the_same_port() {
        let port = free_port();
        let mut child = start_listener(port);
        let first = child.id();

        let told = perform(
            &Target::Process {
                pid: first,
                port: Some(port),
            },
            Op::Restart,
        )
        .expect("restart");

        assert!(
            wait_for_listener(port, Duration::from_secs(5)),
            "nothing is serving :{port} after the restart — {told}"
        );
        let second: u32 = told
            .split_whitespace()
            .nth(3)
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("no new pid in {told:?}"));
        assert_ne!(second, first, "that is the same process: {told}");

        let log = log_dir().expect("a state directory").join("hold-port.log");
        assert!(log.exists(), "{} was not written", log.display());

        let _ = Command::new("kill")
            .args(["-KILL", &second.to_string()])
            .status();
        let _ = child.wait();
        let _ = std::fs::remove_file(&log);
    }

    /// A process we start and never reap is a zombie, not a running process.
    /// Waiting for its table entry to disappear would wait for the full grace
    /// period and then report a restart that had in fact already worked.
    #[test]
    fn a_zombie_counts_as_exited() {
        let port = free_port();
        let child = start_listener(port);
        let pid = child.id();
        // Deliberately not reaped: `child` is dropped without a wait.
        assert!(super::signal(pid, "KILL").is_ok());
        assert!(
            wait_for_exit(pid, Duration::from_secs(3)),
            "a zombie was mistaken for a running process"
        );
        let mut child = child;
        let _ = child.wait();
    }

    #[test]
    fn a_freed_port_is_seen_as_free() {
        let port = free_port();
        assert!(
            !wait_for_listener(port, Duration::from_millis(300)),
            "reported a listener on :{port} with nothing there"
        );
    }

    #[test]
    fn init_is_never_signalled() {
        for pid in [0, 1] {
            let err = super::signal(pid, "TERM").unwrap_err();
            assert!(err.contains("refusing"), "{err}");
        }
    }

    #[test]
    fn a_log_is_named_after_the_command_not_the_pid() {
        let plan = Plan {
            argv: vec!["/opt/homebrew/bin/node".into(), "index.ts".into()],
            cwd: "/tmp".into(),
            env: vec![],
        };
        let path = log_path(&plan).expect("a log path");
        assert_eq!(path.file_name().unwrap(), "node.log");
        assert!(plan.describe().contains("index.ts"), "{}", plan.describe());
    }
}
