//! Where a scan spends its time.
//!
//!     cargo run --release --example breakdown
//!
//! Kept because the numbers, not the guesses, decided every optimisation in
//! this project: `lsof` turned out to be 98% of a scan, which is why quarry now
//! asks the kernel directly.

use std::time::Instant;

use quarry::source::{ProcessSource, SocketSource};

fn bench(label: &str, runs: u32, mut f: impl FnMut() -> usize) {
    let _ = f();
    let started = Instant::now();
    let mut n = 0;
    for _ in 0..runs {
        n = f();
    }
    println!("{label:<26}{:>10.3?}  -> {n}", started.elapsed() / runs);
}

fn main() {
    #[cfg(target_os = "macos")]
    bench("native libproc", 5, || {
        quarry::darwin::Native
            .listening()
            .map(|s| s.len())
            .unwrap_or(0)
    });

    bench("lsof", 5, || {
        quarry::lsof::Lsof.listening().map(|s| s.len()).unwrap_or(0)
    });

    let socks = quarry::lsof::Lsof.listening().unwrap_or_default();
    let pids: Vec<u32> = {
        let mut v: Vec<u32> = socks.iter().map(|s| s.pid).collect();
        v.sort_unstable();
        v.dedup();
        v
    };

    let mut procs = quarry::procs::SysProcesses::new();
    bench("sysinfo refresh", 5, || {
        procs.refresh(&pids);
        pids.len()
    });

    let cwds: Vec<std::path::PathBuf> = pids
        .iter()
        .filter_map(|p| procs.info(*p))
        .filter_map(|i| i.cwd)
        .collect();

    let mut resolver = quarry::repo::Resolver::new();
    let started = Instant::now();
    for c in &cwds {
        let _ = resolver.resolve(c);
    }
    println!(
        "{:<26}{:>10.3?}  -> {}",
        "repo resolve (cold)",
        started.elapsed(),
        cwds.len()
    );
    let started = Instant::now();
    for c in &cwds {
        let _ = resolver.resolve(c);
    }
    println!(
        "{:<26}{:>10.3?}",
        "repo resolve (cached)",
        started.elapsed()
    );

    let mut engine = quarry::engine::Engine::live();
    let started = Instant::now();
    let report = engine.scan().expect("scan");
    let cold = started.elapsed();
    let started = Instant::now();
    let _ = engine.scan().expect("scan");
    println!(
        "{:<26}{:>10.3?}  -> {} services",
        "whole scan (cold)",
        cold,
        report.servers.len()
    );
    println!("{:<26}{:>10.3?}", "whole scan (warm)", started.elapsed());
}
