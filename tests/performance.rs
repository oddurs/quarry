//! Performance guards.
//!
//! These are not micro-benchmarks; they are budgets. Each one is set far above
//! what the code costs today, so it never fails for noise, and fails loudly if
//! something becomes an order of magnitude slower. The numbers printed by
//! `cargo test --test performance -- --nocapture` are the useful part.

mod support;

use std::time::{Duration, Instant};

use quarry::app::App;
use quarry::model::{Health, Kind, Server};
use quarry::testkit;
use quarry::theme::Theme;
use quarry::ui;
use support::Rng;

/// A machine with far more listening services than a real one.
fn many(n: usize) -> Vec<Server> {
    let mut rng = Rng::new(4);
    let repos = [
        "acme-web",
        "acme-api",
        "almanac",
        "orchard",
        "seedling",
        "a-repository-with-a-rather-long-name",
    ];
    (0..n)
        .map(|i| {
            let port = 1024 + (i as u16 % 60000);
            let mut b = testkit::server(port, "node")
                .cmdline(
                    "next-server (v16.3.4) /Users/someone/Code/thing/node_modules/.bin/next dev",
                )
                .kind(*rng.pick(&[Kind::Web, Kind::Api, Kind::Database, Kind::System]))
                .health(testkit::ok(rng.next_u64() % 400, Some("A page title")));
            if rng.chance(2) {
                b = b.repo(rng.pick(&repos), "main");
            }
            let mut s = b.build();
            s.pid = 1000 + i as u32;
            s
        })
        .collect()
}

/// A debug build runs this code roughly ten times slower, and CI runs the suite
/// in both profiles. Scaling the budget keeps one number in the source — the
/// one that matters, from a release build — without the debug run failing for a
/// reason that has nothing to do with the code.
///
/// The ratio guards below are deliberately *not* scaled: a ratio holds in both
/// profiles, which is what makes it the better kind of assertion.
fn budget(release: Duration) -> Duration {
    if cfg!(debug_assertions) {
        release * 10
    } else {
        release
    }
}

fn time<T>(label: &str, iterations: usize, mut f: impl FnMut() -> T) -> Duration {
    // One warm-up, so a lazy allocation is not counted as the cost.
    let _ = f();
    let started = Instant::now();
    for _ in 0..iterations {
        let _ = f();
    }
    let total = started.elapsed();
    let each = total / iterations as u32;
    println!("  {label:<44} {:>9.3?} each  ({iterations} runs)", each);
    each
}

#[test]
fn ingesting_a_scan_is_cheap() {
    println!("\ningest:");
    for n in [50usize, 500, 2000] {
        let servers = many(n);
        let mut app = App::new();
        let each = time(&format!("{n} services"), 20, || {
            app.ingest(servers.clone());
        });
        // Generous: this runs once every six seconds.
        assert!(
            each < budget(Duration::from_millis(4)),
            "{n} services took {each:?} to ingest"
        );
    }
}

#[test]
fn filtering_keeps_up_with_typing() {
    println!("\nfilter:");
    let mut app = App::new();
    app.ingest(many(2000));
    let each = time("2000 services, one keystroke", 50, || {
        app.search = "next".into();
        app.rebuild();
    });
    // A keystroke that takes longer than a frame is a keystroke you feel.
    assert!(
        each < budget(Duration::from_micros(600)),
        "filtering took {each:?}"
    );
}

/// A frame draws what is on screen, not what is on the machine. Handing the
/// list widget every row so it can display forty of them made the cost of a
/// frame scale with the number of services.
#[test]
fn a_frame_costs_the_same_on_a_busy_machine() {
    let mut small = App::new();
    small.ingest(many(30));
    let mut large = App::new();
    large.ingest(many(4000));

    let a = time("30 services", 200, || {
        ui::render_frame(&mut small, 160, 50, 0)
    });
    let b = time("4000 services", 200, || {
        ui::render_frame(&mut large, 160, 50, 0)
    });
    assert!(
        b < a * 3,
        "a frame took {b:?} with 4000 services and {a:?} with 30 — the cost is \
         scaling with the machine rather than the window"
    );
}

#[test]
fn a_frame_is_drawn_in_well_under_a_tick() {
    println!("\nrender:");
    for (label, n) in [
        ("a typical machine (30)", 30usize),
        ("a busy one (2000)", 2000),
    ] {
        let mut app = App::new();
        app.theme = Theme::resolve("gotham").expect("theme");
        app.ingest(many(n));
        // The frame itself, not the test's conversion of it into text.
        let each = time(label, 200, || ui::render_frame(&mut app, 160, 50, 0));
        // The event loop ticks every 100ms; a frame must be a rounding error.
        assert!(
            each < budget(Duration::from_millis(2)),
            "{label} took {each:?}"
        );
    }
}

#[test]
fn applying_probe_results_is_cheap() {
    println!("\nhealth updates:");
    let servers = many(2000);
    let mut app = App::new();
    app.ingest(servers.clone());
    let pids: Vec<(u32, u16)> = servers.iter().map(|s| (s.pid, s.primary_port())).collect();

    let mut i = 0usize;
    let each = time("one result, 2000 services", 2000, || {
        let (pid, port) = pids[i % pids.len()];
        i += 1;
        app.apply_health(pid, port, Health::Closed, None);
    });
    // Two thousand of these arrive within a couple of seconds of every scan.
    // This used to rebuild every group on every result, which was 188µs each —
    // 0.4 seconds of work per scan on a busy machine.
    assert!(
        each < budget(Duration::from_micros(2)),
        "a single health update took {each:?}"
    );
}

/// A probe that heard something re-identifies the service against the whole
/// signature table. That is real work, and it is bounded: it happens only when
/// the probe learned something the scan did not.
#[test]
fn re_identifying_from_new_evidence_is_bounded() {
    println!("\nre-identification:");
    let servers = many(2000);
    let mut app = App::new();
    app.ingest(servers.clone());
    let keys: Vec<(u32, u16)> = servers.iter().map(|s| (s.pid, s.primary_port())).collect();

    let mut i = 0usize;
    let each = time("one result carrying a banner", 2000, || {
        let (pid, port) = keys[i % keys.len()];
        i += 1;
        app.apply_health(pid, port, Health::Closed, Some(b"SSH-2.0-OpenSSH".to_vec()));
    });
    assert!(
        each < budget(Duration::from_micros(30)),
        "scoring the signature table took {each:?}"
    );
}

#[test]
fn a_real_scan_stays_within_budget() {
    println!("\nlive scan:");
    let mut engine = quarry::engine::Engine::live();
    let started = Instant::now();
    let first = engine.scan().expect("scan");
    let cold = started.elapsed();

    let started = Instant::now();
    let _ = engine.scan().expect("scan");
    let warm = started.elapsed();

    println!(
        "  {:<44} {cold:>9.3?}",
        format!("cold ({} services)", first.servers.len())
    );
    println!("  {:<44} {warm:>9.3?}", "warm (repo cache hot)");
    // The native socket source made this about twenty times faster than the
    // `lsof` path it replaced. The budget is set to catch a silent fall back to
    // `lsof`, which would show up as tens of milliseconds rather than a few.
    assert!(
        cold < Duration::from_millis(600),
        "a cold scan took {cold:?}"
    );
    assert!(
        warm < Duration::from_millis(400),
        "a warm scan took {warm:?}"
    );
}

/// The whole point of the native socket source.
#[cfg(target_os = "macos")]
#[test]
fn the_native_socket_source_beats_lsof_by_an_order_of_magnitude() {
    use quarry::source::SocketSource;

    let mut native = quarry::darwin::Native;
    let mut lsof = quarry::lsof::Lsof;

    // Warm both, then measure.
    let _ = native.listening();
    let _ = lsof.listening();

    let started = Instant::now();
    let ours = native.listening().expect("native");
    let native_time = started.elapsed();

    let started = Instant::now();
    let theirs = lsof.listening().expect("lsof");
    let lsof_time = started.elapsed();

    println!("\nsocket source:");
    println!(
        "  {:<44} {native_time:>9.3?}  ({} sockets)",
        "native (libproc)",
        ours.len()
    );
    println!(
        "  {:<44} {lsof_time:>9.3?}  ({} sockets)",
        "lsof",
        theirs.len()
    );

    assert!(
        native_time * 5 < lsof_time,
        "native {native_time:?} vs lsof {lsof_time:?} — the native path is not paying for itself"
    );
    assert!(!ours.is_empty(), "the native source found nothing at all");
}
