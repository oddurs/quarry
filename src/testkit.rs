//! Builders shared by unit and integration tests.
//!
//! Kept in the library rather than in `tests/` so both can use it, and so a
//! change to `Server` breaks the builders in one place instead of five.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::Duration;

use crate::model::{Health, Kind, Listener, Repo, Server};
use crate::source::{ProcInfo, RawSocket};

pub fn listener(port: u16) -> Listener {
    Listener::tcp(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

pub fn socket(pid: u32, command: &str, port: u16) -> RawSocket {
    RawSocket {
        pid,
        command: command.to_string(),
        user: "tester".to_string(),
        listener: listener(port),
    }
}

pub fn proc_info(cmdline: &str, cwd: Option<&str>) -> ProcInfo {
    ProcInfo {
        cmdline: cmdline.to_string(),
        name: cmdline.split_whitespace().next().unwrap_or("").to_string(),
        exe: None,
        cwd: cwd.map(PathBuf::from),
        ppid: Some(1),
        started_at: 1_700_000_000,
        cpu: 1.5,
        mem: 128 * 1024 * 1024,
    }
}

pub fn ok(ms: u64, title: Option<&str>) -> Health {
    served(200, ms, title, Some("test"))
}

/// A response with a chosen `Server` header, for fixtures that end up in front
/// of people.
pub fn served(status: u16, ms: u64, title: Option<&str>, server: Option<&str>) -> Health {
    Health::Http {
        status,
        scheme: "http",
        latency: Duration::from_millis(ms),
        server: server.map(str::to_string),
        title: title.map(str::to_string),
        is_html: title.is_some(),
    }
}

pub fn status(code: u16, ms: u64) -> Health {
    Health::Http {
        status: code,
        scheme: "http",
        latency: Duration::from_millis(ms),
        server: None,
        title: None,
        is_html: false,
    }
}

/// A fully-formed service, so tests can say what they mean and leave the rest.
pub struct ServerBuilder {
    server: Server,
}

pub fn server(port: u16, command: &str) -> ServerBuilder {
    ServerBuilder {
        server: Server {
            pid: 10_000 + port as u32,
            ppid: Some(1),
            command: command.to_string(),
            cmdline: command.to_string(),
            exe: Some(PathBuf::from(format!("/usr/bin/{command}"))),
            user: "dev".into(),
            cwd: None,
            listeners: vec![listener(port)],
            repo: None,
            kind: Kind::Other,
            service: None,
            health_path: None,
            uri: None,
            note: None,
            handshake: None,
            banner: None,
            evidence: Vec::new(),
            container: None,
            started_at: 1_700_000_000,
            cpu: 0.5,
            mem: 64 * 1024 * 1024,
            health: Health::Unknown,
        },
    }
}

impl ServerBuilder {
    pub fn repo(mut self, name: &str, branch: &str) -> Self {
        self.server.repo = Some(Repo {
            name: name.to_string(),
            root: PathBuf::from(format!("/src/{name}")),
            branch: Some(branch.to_string()),
            remote: Some(format!("acme/{name}")),
        });
        self.server.cwd = Some(PathBuf::from(format!("/src/{name}")));
        self
    }

    pub fn kind(mut self, kind: Kind) -> Self {
        self.server.kind = kind;
        self
    }

    /// The name a signature gave it, as distinct from the process running it.
    pub fn service(mut self, name: &str) -> Self {
        self.server.service = Some(name.to_string());
        self
    }

    pub fn health(mut self, health: Health) -> Self {
        self.server.health = health;
        self
    }

    pub fn cmdline(mut self, cmdline: &str) -> Self {
        self.server.cmdline = cmdline.to_string();
        self
    }

    pub fn ports(mut self, ports: &[u16]) -> Self {
        self.server.listeners = ports.iter().map(|p| listener(*p)).collect();
        self
    }

    pub fn build(self) -> Server {
        self.server
    }
}

/// A believable developer machine, for documentation and the landing page.
///
/// Deliberately synthetic: rendering a real scan into a public web page would
/// publish the names of whatever the author happens to be working on.
pub fn demo() -> Vec<Server> {
    let mut out = vec![
        server(3000, "node")
            .cmdline("next-server (v16.3.4)")
            .repo("orchard", "main")
            .kind(Kind::Web)
            .service("Next.js")
            .health(served(200, 9, Some("Orchard — Dashboard"), Some("Next.js")))
            .build(),
        server(3001, "node")
            .cmdline("node scripts/worker.js")
            .repo("orchard", "main")
            .kind(Kind::Queue)
            .service("BullMQ worker")
            .health(Health::Open {
                latency: Duration::from_micros(300),
            })
            .build(),
        server(5432, "postgres")
            .cmdline("postgres -D /opt/homebrew/var/postgres")
            .repo("orchard", "main")
            .kind(Kind::Database)
            .service("PostgreSQL")
            .health(Health::Open {
                latency: Duration::from_micros(210),
            })
            .build(),
        server(8000, "python3")
            .cmdline("python3 -m uvicorn app.main:app --reload")
            .repo("ledger-api", "feat/invoices")
            .kind(Kind::Api)
            .service("uvicorn")
            .health(served(200, 24, None, Some("uvicorn")))
            .build(),
        server(9090, "prometheus")
            .cmdline("prometheus --config.file=prometheus.yml")
            .repo("ledger-api", "feat/invoices")
            .kind(Kind::Metrics)
            .service("Prometheus")
            .health(served(
                200,
                6,
                Some("Prometheus Time Series Collection"),
                Some("Prometheus"),
            ))
            .build(),
        server(5173, "node")
            .cmdline("vite")
            .repo("almanac", "main")
            .kind(Kind::Web)
            .service("Vite")
            .health(served(200, 4, Some("Almanac"), Some("Vite")))
            .build(),
        server(9229, "node")
            .cmdline("node --inspect scripts/debug.js")
            .repo("almanac", "main")
            .kind(Kind::Debug)
            .service("Node inspector")
            .health(ok(3, None))
            .build(),
        server(16686, "jaeger")
            .cmdline("jaeger-all-in-one")
            .kind(Kind::Metrics)
            .service("Jaeger")
            .health(status(503, 41))
            .build(),
        server(6379, "redis-server")
            .cmdline("redis-server *:6379")
            .kind(Kind::Cache)
            .service("Redis")
            .health(Health::Open {
                latency: Duration::from_micros(180),
            })
            .build(),
        server(6006, "node")
            .cmdline("storybook dev -p 6006")
            .repo("almanac", "main")
            .kind(Kind::Web)
            .service("Storybook")
            .health(served(200, 18, Some("Almanac — Storybook"), None))
            .build(),
        server(7700, "meilisearch")
            .cmdline("meilisearch --db-path ./data.ms")
            .repo("ledger-api", "feat/invoices")
            .kind(Kind::Search)
            .service("Meilisearch")
            .health(served(200, 5, None, Some("Meilisearch")))
            .build(),
        server(8025, "mailpit")
            .cmdline("mailpit")
            .kind(Kind::Mail)
            .service("Mailpit")
            .health(served(200, 8, Some("Mailpit"), None))
            .build(),
        server(4040, "ngrok")
            .cmdline("ngrok http 3000")
            .kind(Kind::Tunnel)
            .service("ngrok")
            .health(ok(11, None))
            .build(),
    ];
    // The container rows come from a Compose project rather than a repository.
    for (port, name, kind, service, health) in [
        (
            15432u16,
            "db",
            Kind::Database,
            "PostgreSQL",
            Health::Open {
                latency: Duration::from_micros(240),
            },
        ),
        (19000, "minio", Kind::Storage, "MinIO", ok(7, None)),
    ] {
        let mut s = server(port, "com.docker.backend")
            .cmdline("com.docker.backend services")
            .kind(kind)
            .service(service)
            .health(health)
            .build();
        s.container = Some(crate::docker::Container {
            name: format!("harbour-{name}-1"),
            image: if name == "db" {
                "postgres:16".into()
            } else {
                "minio/minio".into()
            },
            state: "running".into(),
            health: Some("healthy".into()),
            project: Some("harbour".into()),
            service: Some(name.into()),
            working_dir: None,
        });
        out.push(s);
    }
    out
}

/// The scenario the snapshot tests render: a couple of repos, a worktree, a
/// database with no repo at all, and one thing that is broken.
pub fn scenario() -> Vec<Server> {
    vec![
        server(3000, "node")
            .cmdline("next-server (v16.3.4)")
            .repo("acme-web", "main")
            .kind(Kind::Web)
            .health(ok(12, Some("Acme — Dashboard")))
            .build(),
        server(3001, "node")
            .cmdline("node /src/acme-web/scripts/worker.js")
            .repo("acme-web", "main")
            .kind(Kind::Api)
            .health(status(503, 8))
            .build(),
        server(8000, "python3")
            .cmdline("python3 -m uvicorn app.main:app --reload")
            .repo("acme-api", "feat/billing")
            .kind(Kind::Api)
            .health(ok(31, None))
            .build(),
        server(5432, "postgres")
            .cmdline("/opt/homebrew/bin/postgres -D /opt/homebrew/var/postgres")
            .kind(Kind::Database)
            .health(Health::Open {
                latency: Duration::from_micros(400),
            })
            .build(),
        server(6379, "redis-server")
            .cmdline("redis-server *:6379")
            .kind(Kind::Cache)
            .health(Health::Closed)
            .build(),
    ]
}

/// Scripted health answers matching [`scenario`], keyed by port.
pub fn scenario_health() -> HashMap<u16, Health> {
    scenario()
        .into_iter()
        .map(|s| (s.primary_port(), s.health))
        .collect()
}
