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

/// 0048 — a container runtime publishes every port from one process, so
/// grouping by pid alone put a whole stack on one row.
mod containers_on_one_process {
    use super::*;
    use quarry::docker::{Containers, parse};
    use quarry::testkit::socket;

    /// Three containers published by one runtime process — which is what every
    /// existing fixture missed by having at most one.
    const STACK: &str = r#"[
      {"Id":"aaa","Names":["/stack-postgres-1"],"Image":"postgres:16",
       "State":"running","Status":"Up","Ports":[{"PrivatePort":5432,"PublicPort":55432}]},
      {"Id":"bbb","Names":["/stack-redis-1"],"Image":"redis:7",
       "State":"running","Status":"Up","Ports":[{"PrivatePort":6379,"PublicPort":56379}]},
      {"Id":"ccc","Names":["/stack-nginx-1"],"Image":"nginx:1",
       "State":"running","Status":"Up","Ports":[{"PrivatePort":80,"PublicPort":58080}]}
    ]"#;

    fn runtime_holding(ports: &[u16]) -> Engine {
        let mut procs: HashMap<u32, ProcInfo> = HashMap::new();
        procs.insert(
            9000,
            ProcInfo {
                cmdline: "com.docker.backend".into(),
                name: "com.docker.backend".into(),
                ppid: Some(1),
                ..Default::default()
            },
        );
        let sockets: Vec<_> = ports
            .iter()
            .map(|p| socket(9000, "com.docker.backend", *p))
            .collect();
        Engine::new(
            Box::new(StaticSockets::new(sockets)),
            Box::new(StaticProcesses::new(procs)),
            Box::new(StaticCwds {
                cwds: HashMap::new(),
            }),
        )
    }

    /// The runtime's own listener on 2375 is behind no container at all.
    fn scan() -> Vec<quarry::model::Server> {
        runtime_holding(&[55432, 56379, 58080, 2375])
            .with_containers(parse(STACK, std::path::Path::new("/var/run/docker.sock")))
            .scan()
            .expect("scan succeeds")
            .servers
    }

    /// The container is the service; the forwarder is not.
    #[test]
    fn each_container_gets_its_own_row() {
        let servers = scan();
        let names: Vec<String> = servers.iter().map(|s| s.service_name()).collect();
        assert_eq!(
            servers.len(),
            4,
            "three containers and the runtime's own listener: {names:?}"
        );
        for expected in ["stack-postgres-1", "stack-redis-1", "stack-nginx-1"] {
            assert!(
                names.iter().any(|n| n == expected),
                "{expected} is missing from {names:?}"
            );
        }
    }

    /// Ten of eleven ports used to be swallowed as `+10` on one row.
    #[test]
    fn each_row_carries_only_its_own_ports() {
        for s in scan() {
            let ports: Vec<u16> = s.listeners.iter().map(|l| l.port).collect();
            assert_eq!(
                ports.len(),
                1,
                "{} carries {ports:?}, which belong to different containers",
                s.service_name()
            );
        }
    }

    /// Ports behind no container stay together: that is the runtime's own
    /// listener, and it is one thing.
    #[test]
    fn the_runtimes_own_listener_is_still_the_runtime() {
        let servers = scan();
        let own = servers
            .iter()
            .find(|s| s.container.is_none())
            .expect("the runtime's own row");
        assert_eq!(own.primary_port(), 2375);
    }

    /// A process with no containers behind it is unchanged.
    #[test]
    fn a_plain_process_with_several_ports_stays_one_row() {
        let servers = runtime_holding(&[3000, 3001])
            .with_containers(Containers::default())
            .scan()
            .expect("scan succeeds")
            .servers;
        assert_eq!(servers.len(), 1, "one process, one row");
        assert_eq!(servers[0].listeners.len(), 2);
    }
}

/// A probe result belongs to one service, and a container runtime publishes
/// every port from one process — so several services share a pid.
mod probe_results {
    use quarry::model::{Kind, Server};
    use quarry::testkit::server;

    fn stack() -> Vec<Server> {
        // Three containers, all published by the same runtime process.
        [55432u16, 56379, 58080]
            .iter()
            .map(|port| {
                let mut s = server(*port, "com.docker.backend")
                    .kind(Kind::Container)
                    .build();
                s.pid = 81937;
                s
            })
            .collect()
    }

    #[test]
    fn a_result_lands_on_one_service_even_when_a_pid_is_shared() {
        let servers = stack();
        let matched: Vec<u16> = servers
            .iter()
            .filter(|s| s.answers(81937, 56379))
            .map(|s| s.primary_port())
            .collect();
        assert_eq!(
            matched,
            vec![56379],
            "one probe result was applied to every service sharing the pid"
        );
    }

    /// A pid can be reused between scans, and a stale answer landing on a new
    /// process would be a lie.
    #[test]
    fn a_result_for_another_process_lands_nowhere() {
        assert!(stack().iter().all(|s| !s.answers(4242, 56379)));
    }
}

/// 0049 — for a published port the host process is the runtime, which
/// classifies as `container` and is not wrong about itself. It is simply not
/// the service.
mod containers_are_classified_by_image {
    use super::*;
    use quarry::docker::parse;
    use quarry::model::Kind;
    use quarry::testkit::socket;

    fn scan(image: &str, port: u16) -> quarry::model::Server {
        let mut procs: HashMap<u32, ProcInfo> = HashMap::new();
        procs.insert(
            9000,
            ProcInfo {
                cmdline: "com.docker.backend".into(),
                name: "com.docker.backend".into(),
                ppid: Some(1),
                ..Default::default()
            },
        );
        let body = format!(
            r#"[{{"Id":"aaa","Names":["/stack-thing-1"],"Image":"{image}","State":"running",
                 "Status":"Up","Ports":[{{"PrivatePort":1,"PublicPort":{port}}}]}}]"#
        );
        Engine::new(
            Box::new(StaticSockets::new(vec![socket(
                9000,
                "com.docker.backend",
                port,
            )])),
            Box::new(StaticProcesses::new(procs)),
            Box::new(StaticCwds {
                cwds: HashMap::new(),
            }),
        )
        .with_containers(parse(&body, std::path::Path::new("/var/run/docker.sock")))
        .scan()
        .expect("scan succeeds")
        .servers
        .pop()
        .expect("one service")
    }

    /// A PostgreSQL in a container is a database.
    #[test]
    fn the_taxonomy_reaches_containerised_services() {
        for (image, port, kind) in [
            ("postgres:16-alpine", 55432u16, Kind::Database),
            ("redis:7-alpine", 56379, Kind::Cache),
            ("mongo:7", 57017, Kind::Database),
            ("rabbitmq:3-alpine", 55672, Kind::Queue),
            ("nginx:1-alpine", 58080, Kind::Proxy),
            ("quay.io/minio/minio:latest", 59000, Kind::Storage),
        ] {
            let s = scan(image, port);
            assert_eq!(s.kind, kind, "{image} came out as {:?}", s.kind);
        }
    }

    /// `stack-thing-1` is what the user called it; the image is how quarry
    /// worked out what it is. Those are different questions.
    #[test]
    fn the_displayed_name_is_still_the_containers() {
        assert_eq!(
            scan("postgres:16-alpine", 55432).service_name(),
            "stack-thing-1"
        );
    }

    /// An image the table knows nothing about is still a container.
    #[test]
    fn an_unknown_image_is_still_a_container() {
        let s = scan("acme/bespoke-thing:1", 41234);
        assert_eq!(s.kind, Kind::Container, "{:?}", s.kind);
    }
}
