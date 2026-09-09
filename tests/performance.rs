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

/// Whether an absolute budget is worth enforcing on this build.
///
/// Only in release. A debug build runs this code an order of magnitude slower,
/// and by how much depends on the machine — a shared CI runner is slower again.
/// Scaling by a guessed factor just moves the arbitrary number around: a debug
/// run measured 345µs against a budget scaled to 300µs, which said nothing
/// about the code and failed the build.
///
/// So debug measures and prints, release measures and asserts. CI runs the
/// suite in both, and the release run is where the line is held.
///
/// The ratio guards below apply in both profiles: a ratio needs no scaling,
/// which is what makes it the better kind of assertion.
fn enforced() -> bool {
    !cfg!(debug_assertions)
}

/// A shared CI runner measured two to four times slower than the machine these
/// budgets were written on. Multiplying there keeps the guard meaningful — an
/// order-of-magnitude regression still fails — without the build breaking
/// because somebody else's job was busy on the same host.
fn slack() -> u32 {
    if std::env::var_os("CI").is_some() {
        4
    } else {
        1
    }
}

/// Assert a budget, in release only.
#[track_caller]
fn within(each: Duration, budget: Duration, what: &str) {
    let budget = budget * slack();
    if enforced() {
        assert!(each < budget, "{what}: {each:?}, budget {budget:?}");
    } else if each >= budget {
        println!("  (debug build: {what} at {each:?}, over the {budget:?} release budget)");
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
        within(each, Duration::from_millis(4), "ingesting a scan");
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
    within(each, Duration::from_micros(600), "filtering");
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
        within(each, Duration::from_millis(2), label);
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
    within(each, Duration::from_micros(2), "a single health update");
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
    // The cost is linear in the size of the signature table: 564 entries score
    // in about 60µs, and this budget leaves room for the table to roughly
    // double before anyone need think about indexing it. Per scan it is this
    // multiplied by the number of services that heard something new, which on
    // a real machine is a couple of milliseconds.
    within(
        each,
        Duration::from_micros(200),
        "scoring the signature table",
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
    let ours = native.listening().expect("the native source always works");
    let native_time = started.elapsed();

    // `lsof` is not guaranteed to be there, and on a bare CI runner it exits
    // non-zero with nothing to say. Not having it is the situation the native
    // source exists for, so it is not a failure of this test.
    let started = Instant::now();
    let Ok(theirs) = lsof.listening() else {
        println!("  lsof unavailable here; nothing to compare against");
        assert!(!ours.is_empty(), "the native source found nothing at all");
        return;
    };
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

    // A machine with barely anything listening gives `lsof` nothing to walk,
    // which is the one case where it is not slow. The comparison only means
    // something when there is something to compare.
    if theirs.len() >= 8 {
        assert!(
            native_time * 3 < lsof_time,
            "native {native_time:?} vs lsof {lsof_time:?} over {} sockets — \
             the native path is not paying for itself",
            theirs.len()
        );
    } else {
        println!(
            "  only {} sockets here; too few to compare fairly",
            theirs.len()
        );
    }
    assert!(!ours.is_empty(), "the native source found nothing at all");
}
