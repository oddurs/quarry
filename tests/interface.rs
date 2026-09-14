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

mod arranging {
    use super::*;
    use quarry::model::{GroupBy, SortBy};

    fn keys(app: &App) -> Vec<String> {
        app.groups.iter().map(|g| g.key.clone()).collect()
    }

    #[test]
    fn grouping_cycles_and_says_which_way_round_it_is() {
        let mut app = App::new();
        app.ingest(machine());
        assert_eq!(app.group_by, GroupBy::Project);
        assert_eq!(keys(&app), vec!["unattributed", "acme-web"]);

        app.run(Command::GroupBy);
        assert_eq!(app.group_by, GroupBy::Kind);
        assert_eq!(keys(&app), vec!["metrics", "db", "web"]);
        assert!(app.toast.as_ref().expect("a toast").0.contains("kind"));

        app.run(Command::GroupBy);
        assert_eq!(app.group_by, GroupBy::Nothing);
        assert!(keys(&app).is_empty(), "flat means no headings at all");
        assert_eq!(ports(&app).len(), 3, "every service is still there");

        app.run(Command::GroupBy);
        assert_eq!(app.group_by, GroupBy::Project, "it comes back round");
    }

    /// Folds are remembered by key, and the keys differ in every arrangement.
    /// Carried across, a group folds itself because something with the same
    /// name was folded two modes ago.
    #[test]
    fn regrouping_forgets_what_was_folded() {
        let mut app = App::new();
        app.ingest(machine());
        app.collapsed.insert("acme-web".to_string());
        app.rebuild();
        assert_eq!(ports(&app).len(), 1);

        app.run(Command::GroupBy);
        assert!(app.collapsed.is_empty());
        assert_eq!(ports(&app).len(), 3);
    }

    #[test]
    fn sorting_reorders_within_a_group() {
        let mut app = App::new();
        app.group_by = GroupBy::Nothing;
        app.ingest(vec![
            server(8080, "node").service("Zulu").build(),
            server(3000, "node").service("Alpha").build(),
            server(5000, "node").service("Mike").build(),
        ]);
        assert_eq!(app.sort_by, SortBy::Health);
        assert_eq!(
            ports(&app),
            vec![3000, 5000, 8080],
            "health ties break on port"
        );

        app.run(Command::SortBy);
        assert_eq!(app.sort_by, SortBy::Port);
        assert_eq!(ports(&app), vec![3000, 5000, 8080]);

        app.run(Command::SortBy);
        assert_eq!(app.sort_by, SortBy::Name);
        assert_eq!(ports(&app), vec![3000, 5000, 8080], "Alpha, Mike, Zulu");
    }

    /// Without a total order the list reshuffles between two equal rows on
    /// every scan, which looks like the machine is churning when it is not.
    #[test]
    fn every_order_is_total() {
        let mut app = App::new();
        app.group_by = GroupBy::Nothing;
        let same = || {
            vec![
                server(8080, "node").service("Same").build(),
                server(3000, "node").service("Same").build(),
            ]
        };
        for _ in 0..SortBy::ALL.len() {
            app.run(Command::SortBy);
            app.ingest(same());
            let first = ports(&app);
            app.ingest(same());
            assert_eq!(first, ports(&app), "unstable under {:?}", app.sort_by);
            assert_eq!(first, vec![3000, 8080], "port has to break the tie");
        }
    }

    #[test]
    fn the_pane_says_which_arrangement_it_is_in() {
        let mut app = App::new();
        assert_eq!(app.arrangement(), None, "the default needs no explaining");
        app.run(Command::GroupBy);
        assert_eq!(app.arrangement().as_deref(), Some("by kind"));
        app.run(Command::SortBy);
        assert_eq!(app.arrangement().as_deref(), Some("by kind, port order"));
    }
}

mod highlighting {
    use super::*;

    /// Deliberately small, and read through the background map rather than
    /// the text: a pid like 14000 appears in the detail pane as well as the
    /// row, and matching on the digits found the wrong one.
    fn arrived(theme: &str) -> App {
        let mut app = App::new();
        app.theme = quarry::theme::Theme::resolve(theme).expect("theme resolves");
        app.ingest(vec![server(3000, "node").service("Alpha").build()]);
        app.ingest(vec![
            server(3000, "node").service("Alpha").build(),
            server(4000, "node").service("Bravo").build(),
        ]);
        app
    }

    /// The ground under the row for a given port.
    fn ground_of(app: &mut App, port: u16) -> String {
        let at = app
            .rows
            .iter()
            .position(|r| matches!(r, Row::Server(i) if app.servers[*i].primary_port() == port))
            .expect("a row for that port");
        let map = quarry::ui::render_background_to_string(app, 60, 10, 0);
        map.lines()
            .skip_while(|l| !l.is_empty())
            .filter(|l| !l.is_empty())
            // Past the title, the rule and the pane's top border.
            .nth(3 + at)
            .expect("that row is on screen")
            .to_string()
    }

    /// "Highlight" has to mean more than one character of gutter: the row of a
    /// service that just appeared sits on its own ground, edge to edge.
    #[test]
    fn an_arrival_gets_its_own_ground() {
        let mut app = arrived("gotham");
        app.selected = 0;
        let new = ground_of(&mut app, 4000);
        let old = ground_of(&mut app, 3000);
        assert_ne!(new, old, "both rows are on the same ground");

        let tint = new.trim_matches('a');
        assert!(!tint.is_empty(), "the arrival has no ground of its own");
        assert_eq!(
            tint.chars().collect::<std::collections::HashSet<_>>().len(),
            1,
            "the tint stops short of the edge: {new:?}"
        );
    }

    /// Two highlights on one line is one too many, and the cursor has to win.
    #[test]
    fn the_cursor_beats_the_highlight() {
        let mut app = arrived("gotham");
        app.selected = 0;
        let unselected = ground_of(&mut app, 4000);

        app.selected = app
            .rows
            .iter()
            .position(|r| matches!(r, Row::Server(i) if app.servers[*i].primary_port() == 4000))
            .expect("a row for it");
        let selected = ground_of(&mut app, 4000);
        assert_ne!(selected, unselected, "selection did not override the tint");
    }

    /// A theme that cannot know the ground colour must not tint it. The marker
    /// in the gutter carries the same news without the risk.
    #[test]
    fn a_theme_with_no_ground_of_its_own_only_marks_it() {
        let mut app = arrived("mono");
        app.selected = 0;
        assert!(!app.theme.tints_arrivals());
        assert_eq!(
            ground_of(&mut app, 4000),
            ground_of(&mut app, 3000),
            "mono tinted a row it cannot see"
        );

        let screen = quarry::ui::render_to_string(&mut app, 60, 10, 0);
        let row = screen
            .lines()
            .find(|l| l.contains("Bravo"))
            .expect("the new row");
        assert!(row.contains('+'), "no marker either: {row:?}");
    }

    /// Half a minute, measured from the other end: you start a server, watch
    /// it boot, and switch to quarry.
    #[test]
    fn the_highlight_outlasts_switching_windows() {
        let mut app = arrived("gotham");
        app.selected = 0;
        let still_lit = ground_of(&mut app, 4000);

        // Wind the clock back by pretending it appeared a minute ago.
        for s in app.servers.iter_mut() {
            if let Some(at) = s.appeared {
                s.appeared = at.checked_sub(Duration::from_secs(60));
            }
        }
        let faded = ground_of(&mut app, 4000);
        assert_ne!(still_lit, faded, "the highlight never goes out");
        assert_eq!(
            faded,
            ground_of(&mut app, 3000),
            "and it faded to the usual ground"
        );
    }
}

mod excluding {
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
    fn a_term_can_be_turned_inside_out() {
        assert_eq!(matching("!@db"), vec![3000, 16686]);
        assert_eq!(matching("!:16686"), vec![3000, 5432]);
        assert_eq!(matching("~acme !@web"), vec![5432]);
    }

    #[test]
    fn a_bare_negation_is_not_a_term() {
        assert!(Query::parse("!").is_empty());
        assert!(Query::parse("! !: !@").is_empty());
    }
}

/// The list is a table. Every defect here was one column deciding its own
/// width and pushing the rest of the row off its line.
mod columns {
    use super::*;

    fn crowded() -> Vec<Server> {
        vec![
            server(8384, "syncthing")
                .service("Syncthing")
                .kind(Kind::Storage)
                .health(testkit::served(200, 1, None, None))
                .build(),
            server(5353, "Google Chrome")
                .service("Chrome DevTools Protocol")
                .kind(Kind::Debug)
                .build(),
            server(49967, "firefox").service("firefox").build(),
            // A socket whose name is longer than the whole row.
            server(1, "fresh")
                .unix("/tmp/fresh-501/local-27472-81dd8dbc70.ctrl.sock")
                .service("fresh")
                .build(),
            // And one with more listeners than it shows.
            server(8899, "python3")
                .service("Python http.server")
                .kind(Kind::Web)
                .also_on(8900)
                .health(testkit::served(200, 2, None, None))
                .build(),
        ]
    }

    fn rows_at(width: u16) -> Vec<String> {
        let mut app = App::new();
        app.group_by = quarry::model::GroupBy::Nothing;
        // A unix socket is background unless asked for, and it is the row the
        // column has to survive.
        app.show_all = true;
        app.ingest(crowded());
        let rendered = quarry::ui::render_to_string(&mut app, width, 12, 0);
        let rows: Vec<String> = rendered
            .lines()
            .filter(|l| l.chars().any(|c| "●○✕▲".contains(c)))
            .map(str::to_string)
            .collect();
        assert_eq!(
            rows.len(),
            crowded().len(),
            "at {width} columns some rows did not render:\n{rendered}"
        );
        rows
    }

    /// `Sync…storage`. A name truncated to exactly its column ran straight
    /// into the badge, so the two read as one word.
    #[test]
    fn a_truncated_name_never_touches_what_follows_it() {
        for width in 30..=140u16 {
            for row in rows_at(width) {
                let chars: Vec<char> = row.chars().collect();
                for (i, c) in chars.iter().enumerate() {
                    if *c == '…' {
                        let next = chars.get(i + 1).copied().unwrap_or(' ');
                        assert!(
                            next.is_whitespace() || next == '│',
                            "at {width} columns the truncation runs into the next \
                             column: {row:?}"
                        );
                    }
                }
            }
        }
    }

    /// A unix socket's name is as long as someone felt like making it. Sizing
    /// the first column from it pushed the name, the kind and the status off
    /// their lines for every other row on screen.
    #[test]
    fn one_long_socket_does_not_move_every_other_row() {
        for width in 60..=140u16 {
            let rows = rows_at(width);
            // Where the name begins: past the gutter, the arrival column and
            // the health dot, over the first column's right-alignment padding,
            // over the value itself, and over the one space after it.
            let starts: Vec<usize> = rows
                .iter()
                .map(|r| {
                    let c: Vec<char> = r.chars().collect();
                    let mut i = 4;
                    while i < c.len() && c[i].is_whitespace() {
                        i += 1;
                    }
                    while i < c.len() && !c[i].is_whitespace() {
                        i += 1;
                    }
                    i + 1
                })
                .collect();
            assert!(
                starts.windows(2).all(|w| w[0] == w[1]),
                "at {width} columns the rows do not line up: {rows:#?}"
            );
        }
    }

    /// The name is the row; the kind is an attribute of it that the colour
    /// already half carries. When only one fits, the name wins.
    #[test]
    fn the_name_outlives_the_badge() {
        let narrow = rows_at(74);
        let syncthing = narrow
            .iter()
            .find(|r| r.contains("Sync"))
            .expect("the syncthing row");
        assert!(
            syncthing.contains("Syncthing"),
            "the name was cut to keep a badge: {syncthing:?}"
        );
    }

    #[test]
    fn no_row_is_wider_than_the_terminal() {
        for width in 30..=140u16 {
            for row in rows_at(width) {
                assert_eq!(
                    row.chars().count(),
                    width as usize,
                    "at {width} columns: {row:?}"
                );
            }
        }
    }
}
