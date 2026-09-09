//! Terminal lifecycle. These drive the real binary under a real pty, because
//! the failure they guard against — a closed terminal leaving an orphan
//! spinning at 100% CPU — cannot be reproduced any other way.
//!
//! This file uses `unsafe` for `forkpty`; the library itself denies it.

#![cfg(unix)]

use std::ffi::CString;
use std::time::{Duration, Instant};

struct Child {
    pid: libc::pid_t,
    master: i32,
}

impl Child {
    fn spawn() -> Self {
        Self::spawn_with(&[])
    }

    /// Start quarry attached to a fresh pty sized like a normal terminal.
    ///
    /// `env` entries are `KEY=VALUE`. Every run is isolated from the
    /// developer's own config, so a test cannot pass or fail because of what is
    /// in their home directory.
    fn spawn_with(env: &[&str]) -> Self {
        // Everything the child needs must be allocated before the fork: after
        // it, only async-signal-safe calls are legal in a threaded process.
        let exe = CString::new(env!("CARGO_BIN_EXE_quarry")).expect("exe path");
        let arg0 = CString::new("quarry").expect("argv0");
        let mut env_owned: Vec<CString> = vec![
            CString::new("TERM=xterm-256color").expect("env"),
            CString::new("QUARRY_CONFIG=/nonexistent/quarry-test.toml").expect("env"),
        ];
        for extra in env {
            // A later entry wins, so an explicit QUARRY_CONFIG replaces ours.
            let key = extra.split('=').next().unwrap_or("");
            env_owned.retain(|e| !e.to_string_lossy().starts_with(&format!("{key}=")));
            env_owned.push(CString::new(*extra).expect("env"));
        }
        let argv = [arg0.as_ptr(), std::ptr::null()];
        let mut envp: Vec<*const libc::c_char> = env_owned.iter().map(|e| e.as_ptr()).collect();
        envp.push(std::ptr::null());

        let winsize = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        let mut master: i32 = -1;
        let pid = unsafe {
            libc::forkpty(
                &mut master,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &winsize as *const libc::winsize as *mut libc::winsize,
            )
        };
        assert!(pid >= 0, "forkpty failed");

        if pid == 0 {
            unsafe {
                libc::execve(exe.as_ptr(), argv.as_ptr(), envp.as_ptr());
                libc::_exit(127);
            }
        }
        Self { pid, master }
    }

    /// Let it draw for a while, discarding what it writes.
    fn settle(&self, how_long: Duration) {
        let end = Instant::now() + how_long;
        let mut buf = [0u8; 8192];
        while Instant::now() < end {
            let mut fds = libc::pollfd {
                fd: self.master,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut fds, 1, 100) };
            if ready > 0 {
                let n = unsafe {
                    libc::read(
                        self.master,
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    )
                };
                if n <= 0 {
                    return;
                }
            }
        }
    }

    fn close_terminal(&mut self) {
        if self.master >= 0 {
            unsafe { libc::close(self.master) };
            self.master = -1;
        }
    }

    /// Wait for exit, draining the pty meanwhile.
    ///
    /// The draining matters: a TUI redraws several times a second, and a pty
    /// nobody reads fills up and blocks the writer. A test that stops reading
    /// is testing a wedged terminal, not a wedged program.
    fn wait(&self, limit: Duration) -> Option<Duration> {
        let started = Instant::now();
        let mut buf = [0u8; 8192];
        while started.elapsed() < limit {
            let mut status = 0;
            let r = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if r == self.pid {
                return Some(started.elapsed());
            }
            if self.master >= 0 {
                let mut fds = libc::pollfd {
                    fd: self.master,
                    events: libc::POLLIN,
                    revents: 0,
                };
                if unsafe { libc::poll(&mut fds, 1, 25) } > 0 {
                    unsafe {
                        libc::read(
                            self.master,
                            buf.as_mut_ptr() as *mut libc::c_void,
                            buf.len(),
                        )
                    };
                }
            } else {
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        None
    }

    /// Like `wait`, but keeps everything the child wrote.
    fn wait_capturing(&self, limit: Duration) -> (Option<Duration>, Vec<u8>) {
        let started = Instant::now();
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        let mut exited = None;
        while started.elapsed() < limit {
            if exited.is_none() {
                let mut status = 0;
                if unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) } == self.pid {
                    exited = Some(started.elapsed());
                }
            }
            let mut fds = libc::pollfd {
                fd: self.master,
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut fds, 1, 25) } > 0 {
                let n = unsafe {
                    libc::read(
                        self.master,
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    )
                };
                if n > 0 {
                    out.extend_from_slice(&buf[..n as usize]);
                    continue;
                }
            }
            if exited.is_some() {
                break;
            }
        }
        (exited, out)
    }

    fn write(&self, bytes: &[u8]) {
        let n = unsafe {
            libc::write(
                self.master,
                bytes.as_ptr() as *const libc::c_void,
                bytes.len(),
            )
        };
        assert_eq!(n as usize, bytes.len(), "short write to the pty");
    }

    fn kill(&self) {
        unsafe {
            libc::kill(self.pid, libc::SIGKILL);
            let mut status = 0;
            libc::waitpid(self.pid, &mut status, 0);
        }
    }
}

#[test]
fn closing_the_terminal_ends_the_process() {
    let mut child = Child::spawn();
    child.settle(Duration::from_millis(1200));
    child.close_terminal();

    match child.wait(Duration::from_secs(8)) {
        Some(took) => assert!(
            took < Duration::from_secs(6),
            "took {took:?} to notice the terminal had gone"
        ),
        None => {
            child.kill();
            panic!("orphaned: quarry outlived its terminal");
        }
    }
}

#[test]
fn sigterm_ends_the_process() {
    let mut child = Child::spawn();
    child.settle(Duration::from_millis(1000));
    unsafe { libc::kill(child.pid, libc::SIGTERM) };

    let outcome = child.wait(Duration::from_secs(8));
    child.close_terminal();
    assert!(outcome.is_some(), "SIGTERM did not stop quarry");
}

#[test]
fn sighup_ends_the_process() {
    let mut child = Child::spawn();
    child.settle(Duration::from_millis(1000));
    unsafe { libc::kill(child.pid, libc::SIGHUP) };

    let outcome = child.wait(Duration::from_secs(8));
    child.close_terminal();
    assert!(outcome.is_some(), "SIGHUP did not stop quarry");
}

#[test]
fn quitting_with_q_restores_the_terminal() {
    let mut child = Child::spawn();
    child.settle(Duration::from_millis(1200));

    let q = b"q";
    let written = unsafe { libc::write(child.master, q.as_ptr() as *const libc::c_void, 1) };
    assert_eq!(written, 1, "could not write to the pty");

    let (exited, tail) = child.wait_capturing(Duration::from_secs(8));
    child.close_terminal();

    assert!(exited.is_some(), "q did not quit");
    let tail = String::from_utf8_lossy(&tail);
    assert!(
        tail.contains("\u{1b}[?1049l") || tail.contains("\u{1b}[?47l"),
        "the alternate screen was never left; the terminal would be left wrecked"
    );
}

/// Read whatever the child writes for a while, without waiting for it to exit.
fn capture(child: &Child, how_long: Duration) -> Vec<u8> {
    let end = Instant::now() + how_long;
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    while Instant::now() < end {
        let mut fds = libc::pollfd {
            fd: child.master,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut fds, 1, 50) } > 0 {
            let n = unsafe {
                libc::read(
                    child.master,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n <= 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
    }
    out
}

/// Motion tracking makes a terminal emit a report for every mouse movement.
/// quarry uses clicks and scrolling only, and asking for motion would turn any
/// failure to clean up into an unusable shell.
#[test]
fn startup_never_enables_motion_tracking() {
    let mut child = Child::spawn();
    let start = capture(&child, Duration::from_millis(1200));
    child.write(b"q");
    let _ = child.wait(Duration::from_secs(5));
    child.close_terminal();

    let text = String::from_utf8_lossy(&start);
    assert!(text.contains("\u{1b}[?1000h"), "clicks are not enabled");
    assert!(
        text.contains("\u{1b}[?1006h"),
        "SGR encoding is not requested"
    );
    assert!(
        !text.contains("\u{1b}[?1003h"),
        "any-motion tracking (?1003) was enabled — this floods the terminal"
    );
    assert!(
        !text.contains("\u{1b}[?1002h"),
        "drag tracking (?1002) was enabled and is not used"
    );
}

/// Whatever gets turned on has to come back off, and the check is on the wire
/// rather than on the code, because that is where the user experiences it.
#[test]
fn every_mode_enabled_at_startup_is_disabled_at_exit() {
    let mut child = Child::spawn();
    let start = capture(&child, Duration::from_millis(1200));
    child.write(b"q");
    let (exited, tail) = child.wait_capturing(Duration::from_secs(5));
    child.close_terminal();
    assert!(exited.is_some(), "q did not quit");

    let start = String::from_utf8_lossy(&start).to_string();
    let tail = String::from_utf8_lossy(&tail).to_string();

    let enabled: Vec<String> = start
        .match_indices("\u{1b}[?")
        .filter_map(|(i, _)| {
            let rest = &start[i + 3..];
            let end = rest.find(|c: char| !c.is_ascii_digit())?;
            (rest.as_bytes().get(end) == Some(&b'h')).then(|| rest[..end].to_string())
        })
        .collect();
    assert!(!enabled.is_empty(), "nothing was enabled — did it start?");

    for mode in enabled {
        // The cursor is hidden with ?25l and shown again with ?25h, which is
        // the one mode whose polarity is the other way round.
        if mode == "25" {
            continue;
        }
        assert!(
            tail.contains(&format!("\u{1b}[?{mode}l")),
            "mode ?{mode} was enabled at startup and never disabled on exit"
        );
    }
    assert!(
        tail.contains("\u{1b}[?25h"),
        "the cursor was hidden and never shown again"
    );
}

/// Mouse reports arriving on stdin must not upset it.
#[test]
fn a_mouse_click_does_not_disturb_it() {
    let mut child = Child::spawn();
    let _ = capture(&child, Duration::from_millis(1000));

    // SGR: button 0 pressed then released at column 10, row 6.
    child.write(b"\x1b[<0;10;6M");
    child.write(b"\x1b[<0;10;6m");
    // And a scroll wheel event.
    child.write(b"\x1b[<64;10;6M");
    let _ = capture(&child, Duration::from_millis(400));

    child.write(b"q");
    let exited = child.wait(Duration::from_secs(5));
    child.close_terminal();
    assert!(exited.is_some(), "mouse input left it unable to quit");
}

/// Editing a theme and seeing the result is the one task that is pure trial and
/// error, so it must not need a restart.
#[test]
fn reload_picks_up_an_edited_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "theme = \"night\"\n").expect("write");

    let mut child = Child::spawn_with(&[&format!("QUARRY_CONFIG={}", path.display())]);
    let _ = capture(&child, Duration::from_millis(1200));

    // Change the theme underneath it, then ask for a reload.
    std::fs::write(&path, "theme = \"gotham\"\n").expect("rewrite");
    child.write(b"\x12"); // ctrl-r
    let after = capture(&child, Duration::from_millis(900));

    child.write(b"q");
    let exited = child.wait(Duration::from_secs(5));
    child.close_terminal();
    assert!(exited.is_some(), "it stopped responding after a reload");

    let text = String::from_utf8_lossy(&after);
    assert!(
        text.contains("Gotham"),
        "the reload did not report the new theme:\n{text}"
    );
}

/// A config that cannot be parsed must not take the running screen down.
#[test]
fn reload_survives_a_broken_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "theme = \"night\"\n").expect("write");

    let mut child = Child::spawn_with(&[&format!("QUARRY_CONFIG={}", path.display())]);
    let _ = capture(&child, Duration::from_millis(1200));

    std::fs::write(&path, "theme = \n[[[ nonsense").expect("rewrite");
    child.write(b"\x12");
    let _ = capture(&child, Duration::from_millis(600));

    child.write(b"q");
    let exited = child.wait(Duration::from_secs(5));
    child.close_terminal();
    assert!(
        exited.is_some(),
        "a broken config on reload left it unusable"
    );
}
