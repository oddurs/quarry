//! `--here`: one project, seen from inside it.

use quarry::app::{App, Row};
use quarry::model::{Kind, Scope, Server};
use quarry::testkit::server;
use std::path::PathBuf;

/// The ports actually on screen, in display order.
fn on_screen(app: &App) -> Vec<u16> {
    app.rows
        .iter()
        .filter_map(|r| match r {
            Row::Server(i) => Some(app.servers[*i].primary_port()),
            Row::Group(_) | Row::Launcher(_) => None,
        })
        .collect()
}

/// Three checkouts of one project and one service from somewhere else.
fn machine() -> Vec<Server> {
    vec![
        server(3000, "node")
            .cmdline("next-server (v16.3.4)")
            .repo("acme-web", "main")
            .kind(Kind::Web)
            .build(),
        server(3001, "node")
            .cmdline("next-server (v16.3.4)")
            .worktree("acme-web", "feat/billing")
            .kind(Kind::Web)
            .build(),
        server(5432, "postgres")
            .cmdline("postgres -D /var/lib/postgresql")
            .worktree("acme-web", "feat/billing")
            .kind(Kind::Database)
            .build(),
        server(4000, "node")
            .cmdline("node server.js")
            .repo("unrelated", "main")
            .kind(Kind::Web)
            .build(),
    ]
}

fn scoped_app() -> App {
    let mut app = App::new();
    app.scope = Some(Scope {
        root: PathBuf::from("/src/acme-web"),
        name: "acme-web".into(),
        remote: Some("acme/acme-web".into()),
    });
    app.here = true;
    app.ingest(machine());
    app
}

#[test]
fn every_worktree_of_the_project_is_in_scope_and_nothing_else_is() {
    let app = scoped_app();
    // Grouped by branch, so `feat/billing` comes before `main`.
    assert_eq!(
        on_screen(&app),
        vec![3001, 5432, 3000],
        "a worktree is the same project, and :4000 is not"
    );
}

/// The whole point of the view: inside one repository the project name is on
/// every row and says nothing, so the groups are the branches.
#[test]
fn the_groups_are_worktrees_rather_than_projects() {
    let app = scoped_app();
    let keys: Vec<&str> = app.groups.iter().map(|g| g.key.as_str()).collect();
    assert_eq!(keys, vec!["feat/billing", "main"]);
    assert_eq!(
        app.groups.iter().map(|g| g.count).sum::<usize>(),
        3,
        "every in-scope service landed in a group"
    );
    assert!(
        app.groups.iter().all(|g| g.branch.is_none()),
        "the heading is already the branch; printing it twice is noise"
    );
}

/// Two unrelated checkouts can share a name — `site`, `docs`, `api`. The
/// repository root is what says they are not the same project.
#[test]
fn a_different_repository_with_the_same_name_is_not_this_one() {
    let mine = Scope {
        root: PathBuf::from("/src/acme-web"),
        name: "acme-web".into(),
        remote: None,
    };
    let theirs = Scope {
        root: PathBuf::from("/elsewhere/acme-web"),
        name: "acme-web".into(),
        remote: None,
    };
    let s = server(3000, "node").repo("acme-web", "main").build();
    assert!(s.in_scope(&mine));
    assert!(
        !s.in_scope(&theirs),
        "matched on the name alone — two projects called acme-web became one"
    );
}

#[test]
fn turning_the_scope_off_brings_the_machine_back() {
    let mut app = scoped_app();
    app.toggle_here();
    assert!(!app.here);
    assert_eq!(on_screen(&app).len(), 4);

    app.toggle_here();
    assert!(app.here);
    assert_eq!(on_screen(&app).len(), 3);
}

/// Outside a repository there is nothing to narrow to, and a key that silently
/// does nothing is worse than one that says why.
#[test]
fn asking_to_narrow_outside_a_repository_says_so() {
    let mut app = App::new();
    app.ingest(machine());
    app.toggle_here();
    assert!(!app.here);
    let (text, _, _) = app.toast.as_ref().expect("a toast explaining it");
    assert!(text.contains("not in a git repository"), "{text}");
    assert_eq!(on_screen(&app).len(), 4, "nothing was hidden");
}

/// A detached head has no branch to name the checkout, so the directory does.
#[test]
fn a_checkout_with_no_branch_is_named_by_its_directory() {
    let mut s = server(3000, "node")
        .worktree("acme-web", "feat/billing")
        .build();
    assert_eq!(s.worktree_key(), "feat/billing");

    s.repo.as_mut().expect("a repo").branch = None;
    assert_eq!(
        s.worktree_key(),
        "billing",
        "the checkout directory names it"
    );
}

#[test]
fn the_scope_reads_a_real_repository_from_a_directory_inside_it() {
    // quarry's own checkout, found from wherever the test binary runs.
    let here = Scope::containing(&std::env::current_dir().expect("cwd"))
        .expect("the test suite runs inside a git repository");
    assert!(!here.name.is_empty());
    assert!(
        here.root.join(".git").exists(),
        "{} is not a repository root",
        here.root.display()
    );
}
