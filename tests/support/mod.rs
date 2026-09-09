//! Shared test scaffolding: a snapshot harness and a seeded generator.

#![allow(dead_code)]

use std::path::PathBuf;

/// Compare against a committed golden file.
///
/// `QUARRY_UPDATE_SNAPSHOTS=1 cargo test` rewrites them. Reviewing the diff of
/// a snapshot file is the point — an unexplained change to the screen should be
/// as visible in a pull request as a change to the code.
pub fn assert_snapshot(name: &str, actual: &str) {
    let path = snapshot_path(name);
    let actual = format!("{}\n", actual.trim_end());

    if std::env::var("QUARRY_UPDATE_SNAPSHOTS").is_ok() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("create snapshot dir");
        }
        std::fs::write(&path, &actual).expect("write snapshot");
        return;
    }

    let Ok(expected) = std::fs::read_to_string(&path) else {
        panic!(
            "no snapshot at {}\n\nrun:\n  QUARRY_UPDATE_SNAPSHOTS=1 cargo test\n\nwould have written:\n{actual}",
            path.display()
        );
    };

    if expected != actual {
        panic!(
            "snapshot {name} changed\n\n{}\n\nif this is intended:\n  QUARRY_UPDATE_SNAPSHOTS=1 cargo test",
            diff(&expected, &actual)
        );
    }
}

fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.txt"))
}

/// A line-oriented diff — enough to see what moved without a dependency.
fn diff(expected: &str, actual: &str) -> String {
    let e: Vec<&str> = expected.lines().collect();
    let a: Vec<&str> = actual.lines().collect();
    let mut out = String::new();
    for i in 0..e.len().max(a.len()) {
        match (e.get(i), a.get(i)) {
            (Some(x), Some(y)) if x == y => {}
            (Some(x), Some(y)) => {
                out.push_str(&format!("{:>4} - {x}\n{:>4} + {y}\n", i + 1, i + 1));
            }
            (Some(x), None) => out.push_str(&format!("{:>4} - {x}\n", i + 1)),
            (None, Some(y)) => out.push_str(&format!("{:>4} + {y}\n", i + 1)),
            (None, None) => {}
        }
    }
    if out.is_empty() {
        out.push_str("(only trailing whitespace differs)");
    }
    out
}

/// A tiny deterministic PRNG. Seeded explicitly so a failing case can be
/// replayed from the seed printed in the failure.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(6364136223846793005).wrapping_add(1) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    pub fn range(&mut self, lo: u16, hi: u16) -> u16 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo) as u64) as u16
    }

    pub fn pick<'a, T: ?Sized>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    pub fn chance(&mut self, one_in: u64) -> bool {
        one_in > 0 && self.next_u64().is_multiple_of(one_in)
    }
}
