//! Running an external command without ever hanging on it.
//!
//! `lsof` can block indefinitely on a wedged NFS mount or a stuck process. A
//! TUI that refreshes every six seconds cannot afford that, so every subprocess
//! goes through here: piped output drained on its own thread (a full pipe
//! buffer is its own deadlock), a hard deadline, and a kill on expiry.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub enum ExecError {
    /// The binary is not on PATH, or we were not allowed to run it.
    Spawn(std::io::Error),
    /// It ran past its deadline and was killed.
    Timeout(Duration),
    /// It exited non-zero with nothing useful on stdout.
    Failed { code: Option<i32>, stderr: String },
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Spawn(e) => write!(f, "could not run: {e}"),
            ExecError::Timeout(d) => write!(f, "timed out after {}ms", d.as_millis()),
            ExecError::Failed { code, stderr } => match code {
                Some(c) => write!(f, "exited {c}: {}", first_line(stderr)),
                None => write!(f, "killed by a signal: {}", first_line(stderr)),
            },
        }
    }
}

impl std::error::Error for ExecError {}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

/// Run a command, returning stdout. Non-zero exits are tolerated as long as
/// something came back on stdout — `lsof` routinely exits 1 while still
/// reporting every socket it could see.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> Result<String, ExecError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ExecError::Spawn)?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel::<(String, String)>();
    std::thread::spawn(move || {
        let mut out = String::new();
        let mut err = String::new();
        if let Some(mut s) = stdout {
            let _ = s.read_to_string(&mut out);
        }
        if let Some(mut s) = stderr {
            let _ = s.read_to_string(&mut err);
        }
        let _ = tx.send((out, err));
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => return Err(ExecError::Spawn(e)),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ExecError::Timeout(timeout));
        }
        std::thread::sleep(POLL);
    };

    // The reader finishes as soon as both pipes close, which the exit above
    // guarantees; the timeout is belt and braces against a leaked descriptor.
    let (out, err) = rx
        .recv_timeout(Duration::from_millis(500))
        .unwrap_or_default();

    if out.trim().is_empty() && !status.success() {
        return Err(ExecError::Failed {
            code: status.code(),
            stderr: err,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout() {
        let out = run("echo", &["hello"], Duration::from_secs(2)).expect("echo runs");
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn kills_a_command_that_overruns() {
        let started = Instant::now();
        let err = run("sleep", &["30"], Duration::from_millis(150)).unwrap_err();
        assert!(matches!(err, ExecError::Timeout(_)), "got {err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "kill was not prompt"
        );
    }

    #[test]
    fn reports_a_missing_binary() {
        let err = run("quarry-not-a-real-binary", &[], Duration::from_secs(1)).unwrap_err();
        assert!(matches!(err, ExecError::Spawn(_)), "got {err:?}");
    }

    #[test]
    fn tolerates_a_nonzero_exit_that_still_printed() {
        let out = run(
            "sh",
            &["-c", "echo partial; exit 1"],
            Duration::from_secs(2),
        )
        .expect("partial output is still useful");
        assert_eq!(out.trim(), "partial");
    }

    #[test]
    fn survives_more_output_than_a_pipe_buffer_holds() {
        let out = run(
            "sh",
            &[
                "-c",
                "for i in $(seq 1 20000); do echo aaaaaaaaaaaaaaaaaaaaaaaa; done",
            ],
            Duration::from_secs(20),
        )
        .expect("large output does not deadlock");
        assert_eq!(out.lines().count(), 20000);
    }
}
