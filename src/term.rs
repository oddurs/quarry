//! Owning the terminal, and giving it back no matter how we leave.
//!
//! A TUI that exits without restoring cooked mode leaves the user with an
//! invisible cursor and no echo. There are four ways out — normal quit, a
//! panic, a fatal signal, and a dropped guard — and all four go through here.

use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// Set while the alternate screen is up, so the restore path is idempotent and
/// safe to call from a panic hook that may run after a normal restore.
static RAW: AtomicBool = AtomicBool::new(false);
/// Set by a signal handler; the event loop checks it every tick.
static TERMINATE: AtomicBool = AtomicBool::new(false);

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Button presses and scrolling, reported in SGR encoding.
///
/// Deliberately *not* crossterm's `EnableMouseCapture`, which also turns on
/// `?1002` and `?1003` — drag and any-motion tracking. Those make the terminal
/// send a report for every pixel of mouse movement, which quarry has no use for
/// and which turns a failure to disable them into an unusable shell.
const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1006h";

/// Every mouse mode, including the ones we never turn on: a terminal we
/// inherited may already have them set, and leaving one on is the failure we
/// are trying to prevent.
const MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

/// Everything needed to hand a terminal back in a usable state.
pub const RESET: &[u8] =
    b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l\x1b[?25h\x1b[0m";

/// One `write` straight to the descriptor.
///
/// Not `println!` and not `execute!`: those buffer, and `execute!` gives up on
/// the rest of its sequence if any part of it fails. A restore that gets half
/// written is the bug this is here to avoid.
fn write_raw(bytes: &[u8]) {
    // SAFETY: a plain write of a borrowed buffer to a descriptor we own.
    #[allow(unsafe_code)]
    unsafe {
        libc::write(
            libc::STDOUT_FILENO,
            bytes.as_ptr() as *const libc::c_void,
            bytes.len(),
        );
    }
}

/// RAII ownership of the terminal.
pub struct Guard {
    mouse: bool,
}

impl Guard {
    pub fn new() -> Result<(Self, Tui)> {
        enable_raw_mode()?;
        RAW.store(true, Ordering::SeqCst);
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;
        write_raw(MOUSE_ON);

        install_panic_hook();
        install_signal_handlers();
        install_exit_hook();
        spawn_watchdog();

        let terminal = Terminal::new(CrosstermBackend::new(out))?;
        Ok((Self { mouse: true }, terminal))
    }

    pub fn set_mouse(&mut self, on: bool) -> Result<()> {
        write_raw(if on { MOUSE_ON } else { MOUSE_OFF });
        self.mouse = on;
        Ok(())
    }

    pub fn restore(&mut self) {
        restore_terminal();
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Hand the terminal back. Safe to call from anywhere, any number of times,
/// including a panic hook and an exit hook.
///
/// The escape sequence is written unconditionally rather than only when we
/// believe we still own the terminal. It is idempotent, it costs one syscall,
/// and the alternative — deciding we already restored when we had not — is
/// precisely how a user ends up with a shell full of mouse reports.
pub fn restore_terminal() {
    write_raw(RESET);
    if RAW.swap(false, Ordering::SeqCst) {
        let _ = disable_raw_mode();
    }
}

/// Print the reset sequence and nothing else, for a shell that a *previous*
/// program left in a bad state. `quarry --fix-terminal`.
pub fn print_reset() {
    write_raw(RESET);
    let _ = disable_raw_mode();
}

/// True once a termination signal has arrived.
pub fn terminating() -> bool {
    TERMINATE.load(Ordering::SeqCst)
}

/// Ask the terminal what colour its background is, with OSC 11.
///
/// Must be called in raw mode and before anything else reads stdin, or the
/// reply gets eaten by the event loop. Returns `None` if the terminal does not
/// answer within `timeout`, which is the common case for anything old.
pub fn query_background(timeout: std::time::Duration) -> Option<ratatui::style::Color> {
    if !RAW.load(Ordering::SeqCst) {
        return None; // Cooked mode would line-buffer the reply forever.
    }
    write_raw(b"\x1b]11;?\x1b\\");

    let deadline = std::time::Instant::now() + timeout;
    let mut buf = Vec::with_capacity(64);
    let mut chunk = [0u8; 64];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        let mut fds = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: polling and reading one descriptor we own, with a timeout.
        #[allow(unsafe_code)]
        let ready = unsafe { libc::poll(&mut fds, 1, left.as_millis().min(1000) as i32) };
        if ready <= 0 {
            return None;
        }
        #[allow(unsafe_code)]
        let n = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                chunk.as_mut_ptr() as *mut libc::c_void,
                chunk.len(),
            )
        };
        if n <= 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n as usize]);
        if let Some(color) = parse_osc11(&buf) {
            return Some(color);
        }
        if buf.len() > 512 {
            return None; // Whatever this is, it is not an OSC 11 reply.
        }
    }
}

/// `\e]11;rgb:0a0a/0f0f/1414\e\\` or the BEL-terminated form.
pub fn parse_osc11(bytes: &[u8]) -> Option<ratatui::style::Color> {
    let text = String::from_utf8_lossy(bytes);
    let start = text.find("]11;")? + 4;
    let rest = &text[start..];
    let end = rest
        .find('\x07')
        .or_else(|| rest.find('\x1b'))
        .unwrap_or(rest.len());
    crate::theme::parse_color(rest[..end].trim())
}

/// Whether the terminal looks dark. Asks, and falls back to assuming dark —
/// the overwhelmingly common case — rather than guessing wrong slowly.
pub fn background_is_dark(timeout: std::time::Duration) -> bool {
    // Free and instant where it exists, which is a minority of terminals.
    if let Ok(fgbg) = std::env::var("COLORFGBG")
        && let Some(bg) = fgbg
            .rsplit(';')
            .next()
            .and_then(|v| v.trim().parse::<u8>().ok())
    {
        return !(7..=15).contains(&bg);
    }
    match query_background(timeout) {
        Some(color) => crate::theme::is_dark(color),
        None => true,
    }
}

/// True when the far end of stdin has gone away — the terminal window closed,
/// or the pty master was released.
///
/// This has to be checked *before* reading, not after: crossterm's `read` spins
/// on a closed descriptor rather than returning an error, so once the loop has
/// entered it, nothing gets it back out.
pub fn input_closed() -> bool {
    hung_up(libc::STDIN_FILENO)
}

/// Whether the far end of a descriptor has gone away.
pub fn hung_up(fd: std::os::raw::c_int) -> bool {
    let mut fds = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: a zero-timeout poll over a single descriptor.
    #[allow(unsafe_code)]
    let n = unsafe { libc::poll(&mut fds, 1, 0) };
    n > 0 && (fds.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)) != 0
}

/// A signal handler can only set a flag, and the main loop can only check that
/// flag between iterations — which is no use if it is wedged inside a library
/// call. This thread is the guarantee that a signal is always fatal: it gives
/// the loop a moment to exit cleanly, then takes the process down itself.
fn spawn_watchdog() {
    std::thread::Builder::new()
        .name("quarry-watchdog".into())
        .spawn(|| {
            loop {
                if terminating() {
                    // Give the main loop a moment to leave on its own terms.
                    std::thread::sleep(std::time::Duration::from_millis(250));

                    // Restoring writes to the terminal, and a terminal that has
                    // stopped reading will block that write forever — so it is
                    // a best effort on a thread we are willing to abandon.
                    std::thread::spawn(restore_terminal);
                    std::thread::sleep(std::time::Duration::from_millis(150));

                    // `_exit`, not `process::exit`: the exit hook would run the
                    // same restore, and if that is what is blocking we would
                    // never leave at all. 128 + SIGTERM, by convention.
                    // SAFETY: immediate termination; nothing left to unwind.
                    #[allow(unsafe_code)]
                    unsafe {
                        libc::_exit(143)
                    };
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        })
        .expect("spawn watchdog thread");
}

#[cfg(test)]
pub fn reset_terminate() {
    TERMINATE.store(false, Ordering::SeqCst);
}

extern "C" fn at_exit() {
    restore_terminal();
}

/// Covers every `std::process::exit`, wherever it is called from.
fn install_exit_hook() {
    // SAFETY: registering an exit handler that only writes and restores termios.
    #[allow(unsafe_code)]
    unsafe {
        libc::atexit(at_exit);
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        crate::diag::error("panic", info.to_string());
        previous(info);
    }));
}

extern "C" fn on_signal(_sig: libc::c_int) {
    // Async-signal-safe: a single atomic store. The event loop does the rest.
    TERMINATE.store(true, Ordering::SeqCst);
}

/// SIGINT arrives as a key event in raw mode, but SIGTERM and SIGHUP do not —
/// without these, `kill` or a closed terminal leaves the tty wrecked.
fn install_signal_handlers() {
    // SAFETY: `signal` with a handler that only performs an atomic store is
    // async-signal-safe, and the handler outlives the process.
    #[allow(unsafe_code)]
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGQUIT, on_signal as *const () as libc::sighandler_t);
        // A write to a closed pipe must not kill us mid-render.
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_osc11_reply_parses() {
        let reply = b"\x1b]11;rgb:0a0a/0f0f/1414\x1b\\";
        assert_eq!(
            parse_osc11(reply),
            Some(ratatui::style::Color::Rgb(0x0a, 0x0f, 0x14))
        );
        // The BEL-terminated form, which is what most terminals actually send.
        let bel = b"\x1b]11;rgb:ffff/ffff/ffff\x07";
        assert_eq!(
            parse_osc11(bel),
            Some(ratatui::style::Color::Rgb(255, 255, 255))
        );
        assert_eq!(parse_osc11(b"not a reply"), None);
        assert_eq!(parse_osc11(b""), None);
    }

    #[test]
    fn querying_a_cooked_terminal_returns_immediately() {
        // Under `cargo test` we are not in raw mode; the query must not block.
        let started = std::time::Instant::now();
        let answer = query_background(std::time::Duration::from_secs(5));
        assert_eq!(answer, None);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "took {:?} to decline",
            started.elapsed()
        );
    }

    #[test]
    fn we_never_enable_motion_tracking() {
        let on = String::from_utf8_lossy(MOUSE_ON);
        assert!(on.contains("?1000h"), "clicks must be reported");
        assert!(on.contains("?1006h"), "SGR encoding must be requested");
        assert!(
            !on.contains("?1002h") && !on.contains("?1003h"),
            "motion tracking floods the terminal and quarry does not use it: {on:?}"
        );
    }

    #[test]
    fn the_reset_turns_off_everything_we_could_have_turned_on() {
        let reset = String::from_utf8_lossy(RESET);
        for mode in [
            "?1000l", "?1002l", "?1003l", "?1006l", "?1015l", "?1049l", "?25h",
        ] {
            assert!(reset.contains(mode), "reset omits {mode}: {reset:?}");
        }
        // Anything we enable must appear in the reset, disabled.
        for enabled in String::from_utf8_lossy(MOUSE_ON).split("\u{1b}[") {
            if let Some(mode) = enabled.strip_suffix('h') {
                assert!(
                    reset.contains(&format!("{mode}l")),
                    "we enable {mode}h but never disable it"
                );
            }
        }
    }

    #[test]
    fn restore_is_idempotent() {
        RAW.store(false, Ordering::SeqCst);
        restore_terminal();
        restore_terminal();
    }

    #[test]
    fn hangup_detection_follows_the_far_end_of_a_pipe() {
        let mut fds = [0i32; 2];
        // SAFETY: a pipe into a two-element array, as the call expects.
        #[allow(unsafe_code)]
        let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
        assert_eq!(rc, 0, "could not create a pipe");
        let (read_end, write_end) = (fds[0], fds[1]);

        // Close-on-exec, immediately. Other tests in this binary spawn
        // processes, and a child that inherits the write end keeps the pipe
        // open — so closing it here would not produce a hangup and this test
        // would fail for reasons that have nothing to do with the code.
        // SAFETY: setting a flag on descriptors this test owns.
        #[allow(unsafe_code)]
        unsafe {
            libc::fcntl(read_end, libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(write_end, libc::F_SETFD, libc::FD_CLOEXEC);
        }

        assert!(!hung_up(read_end), "a live pipe must not look hung up");

        // SAFETY: closing a descriptor this test owns.
        #[allow(unsafe_code)]
        unsafe {
            libc::close(write_end)
        };

        // A child forked in the window before FD_CLOEXEC took effect holds the
        // write end until it execs, so allow a moment for that to happen.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !hung_up(read_end) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            hung_up(read_end),
            "closing the writer must show up as a hangup"
        );

        // Deliberately no assertion on the closed read end: another thread in
        // this test binary can be handed the same descriptor number the moment
        // it is released, which would make the check race.
        #[allow(unsafe_code)]
        unsafe {
            libc::close(read_end)
        };
    }

    #[test]
    fn a_signal_raises_the_termination_flag() {
        reset_terminate();
        assert!(!terminating());
        install_signal_handlers();
        // SAFETY: raising SIGTERM at ourselves with our handler installed.
        #[allow(unsafe_code)]
        unsafe {
            libc::raise(libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !terminating() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(terminating(), "SIGTERM did not set the flag");
        reset_terminate();
    }
}
