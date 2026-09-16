//! 0045 — the taxonomy, against software that actually exists.
//!
//! Every claim in this milestone is checkable and none of it was checked
//! against the thing it describes. The unit tests use fixtures quarry's own
//! authors wrote; the live tests use sockets this repository opens. Neither
//! says whether quarry identifies a real PostgreSQL.
//!
//! Ignored by default, because it needs a container runtime and pulls eleven
//! images:
//!
//! ```sh
//! cargo test --test stack -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::process::Command;
use std::time::{Duration, Instant};

use quarry::engine::Engine;
use quarry::model::{Health, Kind, Server};

const COMPOSE: &str = "tests/fixtures/stack/compose.yml";

/// What each published port must be found as.
///
/// The name is the Compose service name, deliberately: a container's name is
/// what the person running it called the thing, and quarry prefers it to
/// anything the signature table can guess from the host process. The kind is
/// the taxonomy, and that *is* guessed — from the image — so it is the real
/// assertion here.
const EXPECTED: &[(u16, &str, Kind)] = &[
    (55432, "postgres", Kind::Database),
    (56379, "redis", Kind::Cache),
    (57017, "mongodb", Kind::Database),
    (51211, "memcached", Kind::Cache),
    (55672, "rabbitmq", Kind::Queue),
    (59000, "minio", Kind::Storage),
    (53000, "grafana", Kind::Metrics),
    (59090, "prometheus", Kind::Metrics),
    (58025, "mailpit", Kind::Mail),
    (58080, "nginx", Kind::Proxy),
];

fn compose(args: &[&str]) -> Result<String, String> {
    let out = Command::new("docker")
        .args(["compose", "-f", COMPOSE])
        .args(args)
        .output()
        .map_err(|e| format!("docker: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Brings the stack down however the test ends, including on a panic.
struct Stack;

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = compose(&["down", "--remove-orphans", "-t", "5"]);
    }
}

/// Scan and probe until every expected service answers, or give up.
///
/// Waiting for the port to be *listening* is not enough and was the first
/// thing this got wrong: a container runtime binds the forwarder as soon as
/// the container is created, which is well before the software inside it is
/// listening. Connecting through it then gets a refusal, and the whole stack
/// reads as down. `compose up --wait` does not help either — most of these
/// images declare no healthcheck, so it returns as soon as they are running.
fn scan_until_ready(engine: &mut Engine) -> Vec<Server> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut last = Vec::new();
    loop {
        last = engine.scan().map(|r| r.servers).unwrap_or(last);
        probe(&mut last);
        let answering = EXPECTED
            .iter()
            .filter(|(port, _, _)| {
                last.iter().any(|s| {
                    s.listeners.iter().any(|l| l.port == *port)
                        && !matches!(s.health, Health::Unknown | Health::Closed)
                })
            })
            .count();
        if answering == EXPECTED.len() || Instant::now() >= deadline {
            return last;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[test]
#[ignore = "needs a container runtime; run with --ignored"]
fn the_taxonomy_holds_against_a_real_stack() {
    if compose(&["version"]).is_err() {
        println!("no docker compose here; nothing to prove against");
        return;
    }

    let _guard = Stack;
    compose(&["up", "-d", "--wait", "--quiet-pull"])
        .or_else(|_| compose(&["up", "-d"]))
        .expect("the stack comes up");

    let mut engine = Engine::live();
    let servers = scan_until_ready(&mut engine);

    let mut probed: BTreeMap<u16, (String, Kind, Health)> = BTreeMap::new();
    for s in &servers {
        for l in &s.listeners {
            probed.insert(l.port, (s.service_name(), s.kind, s.health.clone()));
        }
    }

    println!("\nport    expected              found                 kind        health");
    let mut wrong = Vec::new();
    for (port, name, kind) in EXPECTED {
        match probed.get(port) {
            Some((found, found_kind, health)) => {
                let ok = found.to_lowercase().contains(&name.to_lowercase());
                println!(
                    "{port:<7} {name:<21} {found:<21} {:<11} {}{}",
                    found_kind.label(),
                    health.summary(),
                    if ok && found_kind == kind {
                        ""
                    } else {
                        "   <—"
                    }
                );
                if !ok {
                    wrong.push(format!("{port} is {found:?}, expected {name:?}"));
                }
                if found_kind != kind {
                    wrong.push(format!(
                        "{port} ({name}) is kind {found_kind:?}, expected {kind:?}"
                    ));
                }
                // Discovered and named is most of it; answering is the rest.
                // A service that is up and refusing every connection is not
                // what "first-class support" claims.
                if matches!(health, Health::Unknown | Health::Closed) {
                    wrong.push(format!("{port} ({name}) never answered: {health:?}"));
                }
            }
            None => {
                println!("{port:<7} {name:<21} {:<21} not found", "—");
                wrong.push(format!("{port} ({name}) was not found at all"));
            }
        }
    }

    // The headline number, measured against a stack somebody would run rather
    // than against fixtures we wrote.
    let published: Vec<&Kind> = EXPECTED
        .iter()
        .filter_map(|(port, _, _)| probed.get(port).map(|(_, k, _)| k))
        .collect();
    let unclassified = published.iter().filter(|k| ***k == Kind::Other).count();
    println!(
        "\n{}/{} identified, {} fell through to `other`",
        published.len() - unclassified,
        EXPECTED.len(),
        unclassified
    );

    // 0038's h2c detection was written against a scripted server. This is the
    // first time it meets one somebody else wrote.
    match grpc_speaks_http2() {
        Ok(bytes) => {
            println!("\ngrpc    a real gRPC server answered the preface with {bytes} bytes")
        }
        Err(why) => wrong.push(format!("the gRPC server was not recognised: {why}")),
    }

    assert!(wrong.is_empty(), "\n  {}", wrong.join("\n  "));
}

/// One test, not two: they share a stack, and two `Drop` guards racing to tear
/// it down means whichever finishes first pulls it out from under the other.
fn grpc_speaks_http2() -> Result<usize, String> {
    use std::net::TcpStream;
    let addr: std::net::SocketAddr = "127.0.0.1:59001".parse().expect("an address");
    let stream = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    let reply = quarry::handshake::run("grpc", &stream, Duration::from_secs(3))
        .ok_or("it did not answer the HTTP/2 preface at all")?;
    if !quarry::handshake::confirms("grpc", &reply) {
        return Err(format!(
            "{:02x?} is not a SETTINGS frame",
            &reply[..reply.len().min(16)]
        ));
    }
    Ok(reply.len())
}

/// Probe every service once, as the tool does, and apply the answers.
fn probe(servers: &mut [Server]) {
    use quarry::probe::{NetProber, Pool, Target};
    use std::sync::Arc;

    let pool = Pool::with_workers(Arc::new(NetProber::default()), 8);
    let mut expected = 0;
    for s in servers.iter() {
        if let Some(l) = s.listeners.first() {
            let mut target = Target::from_listener(s.pid, l, s.kind);
            target.path = s.health_path.clone().unwrap_or_else(|| "/".to_string());
            target.handshake = s.handshake.clone();
            if pool.submit(target) {
                expected += 1;
            }
        }
    }
    for o in pool.collect(expected, Duration::from_secs(20)) {
        for s in servers.iter_mut().filter(|s| s.answers(o.pid, o.port)) {
            s.health = o.health.clone();
        }
    }
}
