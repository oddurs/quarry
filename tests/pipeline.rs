//! End to end: captured `lsof` output plus a real directory layout, through the
//! engine, into the list the user sees. No live machine involved, so the same
//! assertions hold on every developer's laptop and in CI.

mod support;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use quarry::app::{App, Row};
use quarry::engine::Engine;
use quarry::lsof::parse_listening;
use quarry::model::Kind;
use quarry::source::{ProcInfo, StaticCwds, StaticProcesses, StaticSockets};
use tempfile::TempDir;

const FIXTURE: &str = include_str!("fixtures/macos_dev_machine.lsof");

/// A work tree at `<tmp>/<name>` with a branch and an origin.
fn make_repo(tmp: &Path, name: &str, branch: &str) -> PathBuf {
    let root = tmp.join(name);
    let git = root.join(".git");
    fs::create_dir_all(&git).expect("create .git");
    fs::write(git.join("HEAD"), format!("ref: refs/heads/{branch}\n")).expect("HEAD");
    fs::write(
        git.join("config"),
        format!("[remote \"origin\"]\n\turl = git@github.com:acme/{name}.git\n"),
    )
    .expect("config");
    root
}

/// A linked worktree of `parent`, checked out somewhere else entirely.
fn make_worktree(tmp: &Path, parent: &Path, id: &str, branch: &str) -> PathBuf {
    let wt_git = parent.join(".git/worktrees").join(id);
    fs::create_dir_all(&wt_git).expect("create worktree git dir");
    fs::write(wt_git.join("HEAD"), format!("ref: refs/heads/{branch}\n")).expect("HEAD");

    let checkout = tmp.join(".worktrees").join(id);
    fs::create_dir_all(&checkout).expect("create checkout");
    fs::write(
        checkout.join(".git"),
        format!("gitdir: {}\n", wt_git.display()),
    )
    .expect("gitdir pointer");
    checkout
}

struct World {
    _tmp: TempDir,
    engine: Engine,
}

fn build_world() -> World {
    let tmp = TempDir::new().expect("tempdir");
    let base = tmp.path().to_path_buf();

    let web = make_repo(&base, "acme-web", "main");
    let wt = make_worktree(&base, &web, "billing", "feat/billing");
    let api = make_repo(&base, "acme-api", "main");

    let mut procs: HashMap<u32, ProcInfo> = HashMap::new();
    procs.insert(
        49019,
        ProcInfo {
            cmdline: "node index.ts".into(),
            name: "node".into(),
            cwd: Some(web.clone()),
            ppid: Some(1),
            started_at: 1_700_000_000,
            mem: 200 * 1024 * 1024,
            ..Default::default()
        },
    );
    procs.insert(
        50123,
        ProcInfo {
            cmdline: "next-server (v16.3.4)".into(),
            name: "node".into(),
            // The same project, checked out as a linked worktree.
            cwd: Some(wt),
            ppid: Some(1),
            ..Default::default()
        },
    );
    procs.insert(
        50124,
        ProcInfo {
            cmdline: "python3 -m uvicorn app.main:app".into(),
            name: "Python".into(),
            cwd: Some(api),
            ..Default::default()
        },
    );
    // 612, 701, 94 and 15292 deliberately have no process record at all —
    // the common case for anything not owned by the current user.

    let engine = Engine::new(
        Box::new(StaticSockets::new(parse_listening(FIXTURE))),
        Box::new(StaticProcesses::new(procs)),
        Box::new(StaticCwds::default()),
    );
    World { _tmp: tmp, engine }
}

#[test]
fn the_fixture_produces_the_expected_services() {
    let mut w = build_world();
    let report = w.engine.scan().expect("scan succeeds");
    assert_eq!(report.servers.len(), 7, "one per listening process");
    assert!(!report.truncated);
}

#[test]
fn a_worktree_groups_under_its_parent_repository() {
    let mut w = build_world();
    let report = w.engine.scan().expect("scan succeeds");

    let names: Vec<String> = report
        .servers
        .iter()
        .filter_map(|s| s.repo.as_ref().map(|r| r.name.clone()))
        .collect();
    assert_eq!(
        names.iter().filter(|n| *n == "acme-web").count(),
        2,
        "the worktree and the main checkout share a group: {names:?}"
    );

    let worktree = report
        .servers
        .iter()
        .find(|s| s.primary_port() == 3000)
        .expect("port 3000");
    let repo = worktree.repo.as_ref().expect("attributed");
    assert_eq!(repo.branch.as_deref(), Some("feat/billing"));
    assert_eq!(
        repo.remote.as_deref(),
        Some("acme/acme-web"),
        "the remote comes from the parent repository"
    );
}

#[test]
fn ipv4_and_ipv6_on_one_port_are_one_row() {
    let mut w = build_world();
    let report = w.engine.scan().expect("scan succeeds");
    let pg = report
        .servers
        .iter()
        .find(|s| s.primary_port() == 5432)
        .expect("postgres");
    assert_eq!(
        pg.listeners.len(),
        1,
        "dual-stack is one listener to the user"
    );
    assert_eq!(pg.kind, Kind::Database);
}

#[test]
fn services_without_a_process_record_still_classify() {
    let mut w = build_world();
    let report = w.engine.scan().expect("scan succeeds");
    let by_port = |p: u16| {
        report
            .servers
            .iter()
            .find(|s| s.primary_port() == p)
            .unwrap_or_else(|| panic!("no service on {p}"))
    };
    assert_eq!(by_port(6379).kind, Kind::Cache);
    assert_eq!(
        by_port(54727).kind,
        Kind::System,
        "rapportd is background noise"
    );
    assert_eq!(
        by_port(15292).kind,
        Kind::System,
        "a desktop app is not a dev server"
    );
}

#[test]
fn the_default_view_hides_background_noise() {
    let mut w = build_world();
    let report = w.engine.scan().expect("scan succeeds");
    let total = report.servers.len();

    let mut app = App::new();
    app.ingest(report.servers);
    let shown = app
        .rows
        .iter()
        .filter(|r| matches!(r, Row::Server(_)))
        .count();
    assert!(shown < total, "system services should be hidden by default");

    app.show_all = true;
    app.rebuild();
    let all = app
        .rows
        .iter()
        .filter(|r| matches!(r, Row::Server(_)))
        .count();
    assert_eq!(all, total, "pressing a shows everything");
    app.check_invariants().expect("state is consistent");
}

#[test]
fn groups_are_ordered_with_real_repos_first() {
    let mut w = build_world();
    let report = w.engine.scan().expect("scan succeeds");
    let mut app = App::new();
    app.show_all = true;
    app.ingest(report.servers);

    let keys: Vec<&str> = app.groups.iter().map(|g| g.key.as_str()).collect();
    let generic = |k: &str| k == "unattributed" || k == "system";
    let first_generic = keys.iter().position(|k| generic(k)).unwrap_or(keys.len());
    assert!(
        keys[..first_generic].iter().all(|k| !generic(k)),
        "named repositories must come first: {keys:?}"
    );
}

#[test]
fn a_scan_is_repeatable() {
    let mut w = build_world();
    let a = w.engine.scan().expect("first scan");
    let b = w.engine.scan().expect("second scan");
    let key = |r: &quarry::engine::ScanReport| {
        r.servers
            .iter()
            .map(|s| format!("{}:{}:{:?}", s.pid, s.primary_port(), s.kind))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&a),
        key(&b),
        "scanning twice must not change the answer"
    );
}
