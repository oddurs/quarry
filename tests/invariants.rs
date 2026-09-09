//! Randomised durability testing.
//!
//! Every case is generated from an explicit seed and the seed is printed on
//! failure, so a discovered bug replays exactly. These tests are looking for
//! panics and broken invariants, not for specific output.

mod support;

use crossterm::event::{KeyCode, KeyModifiers};
use quarry::app::{Action, App};
use quarry::keys::{Command, Keymap};
use quarry::model::{Health, Kind, Server};
use quarry::testkit;
use quarry::theme::Theme;
use quarry::ui;
use support::Rng;

/// Every theme quarry ships. A role that is `Reset` in one and a real colour in
/// another can hide a contrast bug that only appears in one of them.
const THEMES: [&str; 5] = ["auto", "mono", "gotham", "night", "paper"];

const KEYS: [KeyCode; 16] = [
    KeyCode::Down,
    KeyCode::Up,
    KeyCode::PageDown,
    KeyCode::PageUp,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::Enter,
    KeyCode::Backspace,
    KeyCode::Esc,
    KeyCode::Char(' '),
    KeyCode::Char('j'),
    KeyCode::Char('k'),
    KeyCode::Char('a'),
    KeyCode::Char('/'),
    KeyCode::Char('d'),
    KeyCode::Char('?'),
];

const KINDS: [Kind; 8] = [
    Kind::Web,
    Kind::Api,
    Kind::Database,
    Kind::Cache,
    Kind::Proxy,
    Kind::System,
    Kind::DevTool,
    Kind::Other,
];

const NAMES: [&str; 7] = [
    "node",
    "python3",
    "postgres",
    "redis-server",
    "a-very-long-process-name-that-will-not-fit",
    "",
    "日本語のプロセス",
];

const REPOS: [&str; 5] = [
    "acme-web",
    "acme",
    "a-repository-with-an-extremely-long-name-indeed",
    "ünïcodé-repo",
    "x",
];

fn health(rng: &mut Rng) -> Health {
    match rng.below(5) {
        0 => Health::Unknown,
        1 => Health::Closed,
        2 => Health::Open {
            latency: std::time::Duration::from_micros(rng.next_u64() % 5_000_000),
        },
        3 => testkit::ok(rng.next_u64() % 5000, Some("A page title")),
        _ => testkit::status(
            *rng.pick(&[200u16, 301, 404, 418, 500, 502, 999]),
            rng.next_u64() % 900,
        ),
    }
}

fn servers(rng: &mut Rng, n: usize) -> Vec<Server> {
    (0..n)
        .map(|i| {
            let port = rng.range(1, 65535);
            let mut b = testkit::server(port, rng.pick(&NAMES))
                .kind(*rng.pick(&KINDS))
                .health(health(rng))
                .cmdline(rng.pick(&NAMES));
            if rng.chance(2) {
                b = b.repo(
                    rng.pick(&REPOS),
                    if rng.chance(2) {
                        "main"
                    } else {
                        "feat/a-very-long-branch-name-goes-here"
                    },
                );
            }
            if rng.chance(4) {
                b = b.ports(&[
                    port,
                    port.wrapping_add(1).max(1),
                    port.wrapping_add(2).max(1),
                ]);
            }
            let mut s = b.build();
            s.pid = 1000 + i as u32;
            s
        })
        .collect()
}

/// The core property: no sequence of inputs at any size may panic, corrupt the
/// selection, or draw outside the terminal.
/// How many seeds to run. The default keeps the suite fast; CI raises it, and
/// a bug hunt can raise it much further: `QUARRY_FUZZ_SEEDS=100000 cargo test`.
fn seed_count() -> u64 {
    std::env::var("QUARRY_FUZZ_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80)
}

#[test]
fn random_states_and_inputs_stay_consistent() {
    for seed in 0..seed_count() {
        let mut rng = Rng::new(seed);
        let mut app = App::new();
        let n = rng.below(30);
        app.ingest(servers(&mut rng, n));
        app.check_invariants()
            .unwrap_or_else(|e| panic!("seed {seed}: after ingest: {e}"));

        app.theme = Theme::resolve(rng.pick(&THEMES)).expect("built-in themes resolve");
        let width = rng.range(20, 200);
        let height = rng.range(6, 60);

        for step in 0..40 {
            let key = *rng.pick(&KEYS);
            // Never generate the keys with real-world side effects.
            let action = app.handle_key(key, KeyModifiers::NONE);
            assert_ne!(action, Action::Quit, "seed {seed}: unexpected quit");

            app.check_invariants()
                .unwrap_or_else(|e| panic!("seed {seed}, step {step}, key {key:?}: {e}"));

            let screen = ui::render_to_string(&mut app, width, height, step);
            for (i, line) in screen.lines().enumerate() {
                assert!(
                    line.chars().count() <= width as usize,
                    "seed {seed}, step {step}: line {i} is {} wide in a {width} column terminal:\n{line}",
                    line.chars().count()
                );
            }
            assert!(
                screen.lines().count() <= height as usize,
                "seed {seed}: drew {} lines in a {height} row terminal",
                screen.lines().count()
            );

            // A fresh scan can arrive at any moment, including mid-interaction.
            if rng.chance(8) {
                let n = rng.below(20);
                app.ingest(servers(&mut rng, n));
                app.check_invariants()
                    .unwrap_or_else(|e| panic!("seed {seed}: after rescan: {e}"));
            }
            if rng.chance(10) {
                app.scan_failed("synthetic failure".into(), rng.chance(2));
            }
        }
    }
}

/// Sizes a terminal can actually be, including absurd ones.
#[test]
fn renders_at_every_plausible_size() {
    let mut app = App::new();
    app.ingest(testkit::scenario());
    for theme in THEMES {
        app.theme = Theme::resolve(theme).expect("built-in themes resolve");
        for width in [1u16, 2, 5, 10, 20, 40, 60, 80, 120, 200, 400] {
            for height in [1u16, 2, 3, 5, 10, 24, 40, 100] {
                let screen = ui::render_to_string(&mut app, width, height, 0);
                for line in screen.lines() {
                    assert!(
                        line.chars().count() <= width as usize,
                        "{theme} at {width}x{height}: overflowing line {line:?}"
                    );
                }
            }
        }
    }
}

/// Overlays must survive the same abuse.
#[test]
fn overlays_render_at_every_size() {
    for (name, prepare) in [
        ("help", (|a: &mut App| a.help = true) as fn(&mut App)),
        ("diagnostics", |a: &mut App| a.diagnostics = true),
        ("confirm", |a: &mut App| a.ask_kill(true)),
        ("search", |a: &mut App| {
            a.searching = true;
            a.search = "a very long search string indeed".into();
        }),
    ] {
        let mut app = App::new();
        app.ingest(testkit::scenario());
        prepare(&mut app);
        for width in [10u16, 30, 60, 90, 160] {
            for height in [3u16, 8, 20, 50] {
                let screen = ui::render_to_string(&mut app, width, height, 0);
                for line in screen.lines() {
                    assert!(
                        line.chars().count() <= width as usize,
                        "{name} at {width}x{height}: overflowing line {line:?}"
                    );
                }
            }
        }
    }
}

/// Filtering is the one place where the row list and the group counts can
/// disagree, so it gets its own sweep.
#[test]
fn every_filter_leaves_consistent_state() {
    let mut rng = Rng::new(99);
    let mut app = App::new();
    app.ingest(servers(&mut rng, 40));
    for needle in ["", "3", "node", "acme", "db", "ZZZZ", "日本", "500"] {
        app.search = needle.into();
        app.rebuild();
        app.check_invariants()
            .unwrap_or_else(|e| panic!("filter {needle:?}: {e}"));
        let _ = ui::render_to_string(&mut app, 100, 30, 0);
    }
}

/// Collapsing must never strand the cursor where it cannot be moved.
#[test]
fn collapsing_every_group_leaves_the_cursor_usable() {
    let mut app = App::new();
    app.ingest(testkit::scenario());
    let keys: Vec<String> = app.groups.iter().map(|g| g.key.clone()).collect();
    for key in &keys {
        app.collapsed.insert(key.clone());
        app.rebuild();
        app.check_invariants()
            .unwrap_or_else(|e| panic!("after collapsing {key}: {e}"));
    }
    // Everything folded: the cursor must still land on a header and expand it.
    app.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
    app.check_invariants()
        .expect("expanding from a fully folded list");
    assert!(
        app.collapsed.len() < keys.len(),
        "space did not expand a group"
    );
}

/// The guard rail behind the fuzzer: pressing keys must never reach outside the
/// process. Anything with an effect has to come back as an `Action` for the
/// shell to carry out, so a test run can never open a browser or signal a pid.
#[test]
fn no_key_performs_a_side_effect_on_its_own() {
    let every_key: Vec<KeyCode> = (b'a'..=b'z')
        .chain(b'A'..=b'Z')
        .chain(b'0'..=b'9')
        .map(|c| KeyCode::Char(c as char))
        .chain([
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
        ])
        .collect();

    for key in every_key {
        for mods in [
            KeyModifiers::NONE,
            KeyModifiers::CONTROL,
            KeyModifiers::SHIFT,
        ] {
            let mut app = App::new();
            app.ingest(testkit::scenario());
            let action = app.handle_key(key, mods);
            // Effects are values here, not calls. Opening and copying are
            // allowed to be *requested*; signalling must go through a confirm.
            if let Action::Signal { .. } = action {
                panic!("{key:?} with {mods:?} asked to signal a process with no confirmation");
            }
            app.check_invariants()
                .unwrap_or_else(|e| panic!("{key:?} with {mods:?}: {e}"));
        }
    }
}

/// Killing must take two deliberate steps, always.
#[test]
fn signalling_requires_an_explicit_confirmation() {
    let mut app = App::new();
    app.ingest(testkit::scenario());

    assert_eq!(
        app.handle_key(KeyCode::Char('K'), KeyModifiers::NONE),
        Action::None
    );
    assert!(app.confirm.is_some(), "K must raise a confirmation");

    // Anything other than yes cancels.
    let cancelled = app.handle_key(KeyCode::Char('n'), KeyModifiers::NONE);
    assert_eq!(cancelled, Action::None);
    assert!(app.confirm.is_none());

    app.handle_key(KeyCode::Char('K'), KeyModifiers::NONE);
    match app.handle_key(KeyCode::Char('y'), KeyModifiers::NONE) {
        Action::Signal { force, .. } => assert!(!force, "K is SIGTERM, not SIGKILL"),
        other => panic!("expected a signal action, got {other:?}"),
    }
}

/// Enter asks to open — it does not open.
#[test]
fn enter_returns_a_url_rather_than_launching_anything() {
    let mut app = App::new();
    app.ingest(testkit::scenario());
    match app.handle_key(KeyCode::Enter, KeyModifiers::NONE) {
        Action::Open(url) => assert!(url.starts_with("http"), "got {url}"),
        Action::None => {}
        other => panic!("unexpected {other:?}"),
    }
}

/// A rebound keymap must not be able to break the layout: the help overlay is
/// generated from it, and a long key name is a long row.
#[test]
fn a_rebound_keymap_still_fits_the_screen() {
    let mut rng = Rng::new(7);
    let specs = [
        "ctrl-x",
        "alt-q",
        "f12",
        "ctrl-alt-z",
        "space",
        "pgdn",
        "↑",
        "backspace",
    ];
    for seed in 0..40 {
        let mut bindings = std::collections::BTreeMap::new();
        for _ in 0..rng.below(6) {
            let key = rng.pick(&specs).to_string();
            let action = rng.pick(&Command::ALL).name().to_string();
            bindings.insert(key, action);
        }
        let (keymap, _) = Keymap::from_config(&bindings);

        let mut app = App::new();
        app.ingest(testkit::scenario());
        app.keymap = keymap;
        app.help = true;
        for width in [40u16, 70, 100, 160] {
            let screen = ui::render_to_string(&mut app, width, 30, 0);
            for line in screen.lines() {
                assert!(
                    line.chars().count() <= width as usize,
                    "seed {seed} at {width} columns: {line:?}"
                );
            }
        }
    }
}

/// No configuration may produce a keymap that can signal a process directly.
#[test]
fn no_binding_can_reach_a_signal_without_a_confirmation() {
    let mut rng = Rng::new(11);
    let specs = ["x", "K", "X", "ctrl-k", "delete", "q", "enter"];
    for _ in 0..200 {
        let mut bindings = std::collections::BTreeMap::new();
        for _ in 0..rng.below(5) + 1 {
            bindings.insert(
                rng.pick(&specs).to_string(),
                rng.pick(&Command::ALL).name().to_string(),
            );
        }
        let (keymap, _) = Keymap::from_config(&bindings);

        let mut app = App::new();
        app.ingest(testkit::scenario());
        app.keymap = keymap;

        for spec in specs {
            let Some((code, mods)) = quarry::keys::parse_key(spec) else {
                continue;
            };
            let action = app.handle_key(code, mods);
            assert!(
                !matches!(action, Action::Signal { .. }),
                "{spec:?} signalled a process with no confirmation"
            );
            // Clear any confirmation the key raised, so the next key is judged
            // on its own rather than as an answer to a prompt.
            app.confirm = None;
        }
    }
}

/// Whatever the config does, there is always a way out.
#[test]
fn quit_is_always_reachable() {
    let mut rng = Rng::new(13);
    for _ in 0..100 {
        let mut bindings = std::collections::BTreeMap::new();
        for _ in 0..rng.below(8) {
            bindings.insert(
                rng.pick(&["q", "ctrl-c", "esc", "x", "z"]).to_string(),
                rng.pick(&["", "open", "quit", "help", "refresh"])
                    .to_string(),
            );
        }
        let (keymap, _) = Keymap::from_config(&bindings);
        let mut app = App::new();
        app.ingest(testkit::scenario());
        app.keymap = keymap;

        let quit_keys: Vec<String> = quarry::keys::Command::ALL
            .iter()
            .filter(|c| **c == Command::Quit)
            .flat_map(|c| app.keymap.keys_for(*c))
            .collect();
        assert!(!quit_keys.is_empty(), "no way to quit");

        let (code, mods) =
            quarry::keys::parse_key(&quit_keys[0]).expect("a bound key must be nameable");
        assert_eq!(
            app.handle_key(code, mods),
            Action::Quit,
            "bindings {bindings:?} left quit bound to {:?} which did nothing",
            quit_keys[0]
        );
    }
}
