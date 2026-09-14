//! The parts of the screen that decide what you see first.

use quarry::app::{App, Row};
use quarry::keys::Command;
use quarry::lifecycle::Op;
use quarry::model::{Health, Kind, Query, Server};
use quarry::testkit::{self, server};
use std::time::Duration;

fn ports(app: &App) -> Vec<u16> {
    app.rows
        .iter()
        .filter_map(|r| match r {
            Row::Server(i) => Some(app.servers[*i].primary_port()),
            Row::Group(_) => None,
        })
        .collect()
}

fn machine() -> Vec<Server> {
    vec![
        server(3000, "node")
            .repo("acme-web", "main")
            .kind(Kind::Web)
            .service("Next.js")
            .health(testkit::served(200, 9, None, None))
            .build(),
        server(5432, "postgres")
            .repo("acme-web", "main")
            .kind(Kind::Database)
            .service("PostgreSQL")
            .health(Health::Open {
                latency: Duration::from_micros(210),
            })
            .build(),
        // No project, and broken — the row that used to be hardest to reach.
        server(16686, "jaeger")
            .kind(Kind::Metrics)
            .service("Jaeger")
            .health(testkit::status(503, 41))
            .build(),
    ]
}

/// Unattributed services sort last by rank, and a stray broken container is
/// exactly the kind of thing that has no project — so the one row needing
/// attention was reliably the one furthest down.
#[test]
fn a_group_holding_something_broken_sorts_first() {
    let mut app = App::new();
    app.ingest(machine());
    assert_eq!(
        app.groups.first().map(|g| g.trouble),
        Some(1),
        "groups: {:?}",
        app.groups.iter().map(|g| &g.key).collect::<Vec<_>>()
    );
    assert_eq!(ports(&app).first(), Some(&16686));
}

#[test]
fn jumping_to_trouble_opens_the_group_hiding_it() {
    let mut app = App::new();
    app.ingest(machine());
    let broken = app.groups[0].key.clone();
    app.collapsed.insert(broken.clone());
    app.rebuild();
    assert!(!ports(&app).contains(&16686), "the fixture must hide it");

    app.run(Command::NextTrouble);
    assert!(!app.collapsed.contains(&broken), "the group stayed folded");
    assert_eq!(
        app.selected_server().map(|s| s.primary_port()),
        Some(16686),
        "a key that reports trouble it will not show you is worse than no key"
    );
}

#[test]
fn jumping_when_nothing_is_broken_says_so() {
    let mut app = App::new();
    app.ingest(vec![server(3000, "node").repo("acme-web", "main").build()]);
    app.run(Command::NextTrouble);
    let (text, _, _) = app.toast.as_ref().expect("a toast");
    assert!(text.contains("everything is answering"), "{text}");
}

/// The first scan is not news: marking the whole machine as new would be true
/// and useless.
#[test]
fn only_what_arrived_while_quarry_watched_is_marked_new() {
    let mut app = App::new();
    app.ingest(machine());
    assert!(
        app.servers.iter().all(|s| s.appeared.is_none()),
        "the first scan marked the machine as new"
    );

    let mut next = machine();
    next.push(
        server(4000, "node")
            .repo("acme-web", "main")
            .kind(Kind::Web)
            .build(),
    );
    app.ingest(next);

    let new: Vec<u16> = app
        .servers
        .iter()
        .filter(|s| s.appeared.is_some())
        .map(|s| s.primary_port())
        .collect();
    assert_eq!(new, vec![4000], "only the arrival is new");
}

/// The marker goes in a column that was already blank, so an arrival must not
/// shift anything on the line beside it.
#[test]
fn an_arrival_is_marked_without_moving_the_row() {
    let mut app = App::new();
    app.theme = quarry::theme::Theme::resolve("mono").expect("mono resolves");
    app.ingest(machine());
    let before = quarry::ui::render_to_string(&mut app, 90, 12, 0);

    let mut next = machine();
    next.push(
        server(4000, "node")
            .repo("acme-web", "main")
            .kind(Kind::Web)
            .service("Vite")
            .build(),
    );
    app.ingest(next);
    let after = quarry::ui::render_to_string(&mut app, 90, 12, 0);

    let new_row = after
        .lines()
        .find(|l| l.contains("4000"))
        .expect("the new row");
    assert!(new_row.contains('+'), "not marked as new: {new_row:?}");

    // Every row that was there before is unchanged, character for character.
    for old in before.lines().filter(|l| l.contains("3000")) {
        assert!(
            after.lines().any(|l| l == old),
            "an arrival shifted an existing row:\n  {old:?}"
        );
    }
}

/// A departure leaves no row to mark, so it has to be said.
#[test]
fn something_that_stops_is_reported_once() {
    let mut app = App::new();
    app.ingest(machine());
    app.toast = None;

    let fewer: Vec<Server> = machine()
        .into_iter()
        .filter(|s| s.primary_port() != 5432)
        .collect();
    app.ingest(fewer);

    let (text, _, _) = app.toast.as_ref().expect("a toast about the departure");
    assert!(text.contains("stopped"), "{text}");
    assert!(text.contains("5432"), "{text}");
}

#[test]
fn a_quiet_scan_says_nothing() {
    let mut app = App::new();
    app.ingest(machine());
    app.toast = None;
    app.ingest(machine());
    assert!(
        app.toast.is_none(),
        "{:?}",
        app.toast.as_ref().map(|t| &t.0)
    );
}

/// A worktree is a unit people think in. Doing it a row at a time is four
/// confirmations for one intention.
#[test]
fn a_group_can_be_restarted_as_one() {
    let mut app = App::new();
    app.ingest(machine());
    let group = app
        .rows
        .iter()
        .position(|r| matches!(r, Row::Group(g) if app.groups[*g].key == "acme-web"))
        .expect("the acme-web group");
    app.selected = group;

    app.ask(Op::Restart);
    let confirm = app.confirm.as_ref().expect("a confirmation");
    assert!(confirm.prompt.contains("acme-web"), "{}", confirm.prompt);
    assert!(confirm.detail.contains('2'), "{}", confirm.detail);

    match app.resolve_confirm(true) {
        quarry::app::Action::Lifecycle { targets, op } => {
            assert_eq!(targets.len(), 2, "both services in the group");
            assert_eq!(op, Op::Restart);
        }
        other => panic!("expected a lifecycle action, got {other:?}"),
    }
}

mod filtering {
    use super::*;

    fn matching(text: &str) -> Vec<u16> {
        let q = Query::parse(text);
        machine()
            .into_iter()
            .filter(|s| s.satisfies(&q))
            .map(|s| s.primary_port())
            .collect()
    }

    #[test]
    fn a_prefix_narrows_the_question_to_one_field() {
        assert_eq!(matching(":3000"), vec![3000]);
        assert_eq!(matching("@db"), vec![5432]);
        assert_eq!(matching("~acme"), vec![3000, 5432]);
    }

    #[test]
    fn several_terms_narrow_together() {
        assert_eq!(matching("~acme @web"), vec![3000]);
        assert!(
            matching("~acme @metrics").is_empty(),
            "terms have to be an and, or a second term is never worth typing"
        );
    }

    #[test]
    fn plain_words_still_match_anything() {
        assert_eq!(matching("postgres"), vec![5432]);
        assert_eq!(matching(""), vec![3000, 5432, 16686]);
    }

    /// Half-typed input must not mean "match everything but hide this".
    #[test]
    fn a_bare_prefix_is_not_a_term() {
        assert!(Query::parse(":").is_empty());
        assert!(Query::parse("  @  ~ ").is_empty());
    }
}
