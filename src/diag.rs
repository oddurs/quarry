//! Diagnostics: a bounded in-memory ring the UI can show, and an optional log
//! file for after the fact.
//!
//! Nothing here can fail loudly — a diagnostics system that panics is worse
//! than no diagnostics system.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const RING: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Level::Info => write!(f, "info"),
            Level::Warn => write!(f, "warn"),
            Level::Error => write!(f, "error"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    pub level: Level,
    pub at: Instant,
    pub scope: &'static str,
    pub message: String,
}

struct Inner {
    /// A deque, not a Vec: dropping the oldest event from the front of a Vec
    /// shifts every remaining element, on every event we record.
    events: std::collections::VecDeque<Event>,
    warns: usize,
    errors: usize,
    file: Option<File>,
}

fn state() -> &'static Mutex<Inner> {
    static STATE: OnceLock<Mutex<Inner>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(Inner {
            events: std::collections::VecDeque::new(),
            warns: 0,
            errors: 0,
            file: None,
        })
    })
}

/// Point the log at a file. `QUARRY_LOG=/path/to/log` does this from the
/// environment; an unwritable path is reported once and then ignored.
pub fn init_from_env() {
    let Some(path) = std::env::var_os("QUARRY_LOG").map(PathBuf::from) else {
        return;
    };
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => {
            if let Ok(mut s) = state().lock() {
                s.file = Some(file);
            }
            record(Level::Info, "log", format!("logging to {}", path.display()));
        }
        Err(e) => record(
            Level::Warn,
            "log",
            format!("cannot write {}: {e}", path.display()),
        ),
    }
}

pub fn record(level: Level, scope: &'static str, message: impl Into<String>) {
    let message = message.into();
    let Ok(mut s) = state().lock() else {
        // A poisoned diagnostics lock must not take the process down with it.
        return;
    };
    match level {
        Level::Warn => s.warns += 1,
        Level::Error => s.errors += 1,
        Level::Info => {}
    }
    if let Some(file) = s.file.as_mut() {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "{secs} {level} {scope}: {message}");
        let _ = file.flush();
    }
    if s.events.len() == RING {
        s.events.pop_front();
    }
    s.events.push_back(Event {
        level,
        at: Instant::now(),
        scope,
        message,
    });
}

pub fn info(scope: &'static str, message: impl Into<String>) {
    record(Level::Info, scope, message);
}

pub fn warn(scope: &'static str, message: impl Into<String>) {
    record(Level::Warn, scope, message);
}

pub fn error(scope: &'static str, message: impl Into<String>) {
    record(Level::Error, scope, message);
}

/// Most recent first.
pub fn recent(n: usize) -> Vec<Event> {
    let Ok(s) = state().lock() else {
        return Vec::new();
    };
    s.events.iter().rev().take(n).cloned().collect()
}

/// `(warnings, errors)` since start.
pub fn counts() -> (usize, usize) {
    match state().lock() {
        Ok(s) => (s.warns, s.errors),
        Err(_) => (0, 0),
    }
}

/// Tests that assert on the ring have to take this first: the ring is global,
/// and `cargo test` runs them concurrently.
#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
pub fn reset() {
    if let Ok(mut s) = state().lock() {
        s.events.clear();
        s.warns = 0;
        s.errors = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_bounded_and_newest_first() {
        let _guard = test_lock();
        reset();
        for i in 0..RING + 50 {
            record(Level::Info, "test", format!("event {i}"));
        }
        let recent = recent(RING * 2);
        assert!(recent.len() <= RING, "ring grew past its bound");
        assert!(
            recent[0].message.ends_with(&format!("{}", RING + 49)),
            "newest event is not first: {}",
            recent[0].message
        );
    }
}
