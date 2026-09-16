//! What a service *is*, and the heuristics that decide.
//!
//! Intentionally port-first: a process called `node` tells you nothing, but
//! `node` on 5432 and `node` on 5173 are different animals.

use super::{Health, Rules};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Kind {
    Web,
    Api,
    Database,
    Cache,
    Search,
    Queue,
    Proxy,
    Mail,
    Container,
    Ai,
    Metrics,
    Storage,
    Auth,
    Registry,
    Realtime,
    Vector,
    Workflow,
    Debug,
    Tunnel,
    Notebook,
    Emulator,
    Game,
    DevTool,
    System,
    Other,
}

impl Kind {
    /// Parse the name a config file or theme file uses.
    pub fn from_label(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.label() == s.trim())
    }

    /// Every kind, in the order a theme file lists them.
    pub const ALL: [Kind; 25] = [
        Kind::Web,
        Kind::Api,
        Kind::Database,
        Kind::Cache,
        Kind::Search,
        Kind::Queue,
        Kind::Proxy,
        Kind::Mail,
        Kind::Container,
        Kind::Ai,
        Kind::Metrics,
        Kind::Storage,
        Kind::Auth,
        Kind::Registry,
        Kind::Realtime,
        Kind::Vector,
        Kind::Workflow,
        Kind::Debug,
        Kind::Tunnel,
        Kind::Notebook,
        Kind::Emulator,
        Kind::Game,
        Kind::DevTool,
        Kind::System,
        Kind::Other,
    ];

    /// Position in [`Kind::ALL`], used to index a theme's kind palette.
    pub fn index(self) -> usize {
        Kind::ALL
            .iter()
            .position(|k| *k == self)
            .expect("every kind is in Kind::ALL")
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Web => "web",
            Kind::Api => "api",
            Kind::Database => "db",
            Kind::Cache => "cache",
            Kind::Search => "search",
            Kind::Queue => "queue",
            Kind::Proxy => "proxy",
            Kind::Mail => "mail",
            Kind::Container => "container",
            Kind::Ai => "ai",
            Kind::Metrics => "metrics",
            Kind::Storage => "storage",
            Kind::Auth => "auth",
            Kind::Registry => "registry",
            Kind::Realtime => "realtime",
            Kind::Vector => "vector",
            Kind::Workflow => "workflow",
            Kind::Debug => "debug",
            Kind::Tunnel => "tunnel",
            Kind::Notebook => "notebook",
            Kind::Emulator => "emulator",
            Kind::Game => "game",
            Kind::DevTool => "tool",
            Kind::System => "system",
            Kind::Other => "other",
        }
    }

    /// Services we never want to speak HTTP to.
    pub fn speaks_http(self) -> bool {
        !matches!(
            self,
            Kind::Database | Kind::Cache | Kind::Queue | Kind::System
        )
    }

    /// Whether the browser is the right thing to open. A database has a URI but
    /// not a web page, and launching one at a browser is a promise that cannot
    /// be kept.
    pub fn opens_in_a_browser(self) -> bool {
        matches!(
            self,
            Kind::Web
                | Kind::Api
                | Kind::Proxy
                | Kind::Metrics
                | Kind::Storage
                | Kind::Auth
                | Kind::Registry
                | Kind::Workflow
                | Kind::Notebook
                | Kind::Search
                | Kind::Emulator
                | Kind::Container
                | Kind::Other
        )
    }

    /// Infrastructure the user did not start on purpose today.
    pub fn is_background_noise(self) -> bool {
        matches!(self, Kind::System)
    }

    /// Worth a second glance simply for existing: a debugger left attached, or
    /// a tunnel exposing something local to the internet.
    pub fn is_notable(self) -> bool {
        matches!(self, Kind::Debug | Kind::Tunnel)
    }
}

/// An unclassified process that answers HTTP is a web app if it served markup,
/// and an API otherwise. This runs after the probe, so it can only improve.
pub fn refine_kind(current: Kind, health: &Health) -> Kind {
    if current != Kind::Other {
        return current;
    }
    match health {
        Health::Http { is_html: true, .. } => Kind::Web,
        Health::Http { .. } => Kind::Api,
        _ => current,
    }
}

pub fn scheme_for(port: u16) -> &'static str {
    match port {
        443 | 4443 | 8443 | 9443 | 3443 => "https",
        _ => "http",
    }
}

/// Turn an argv into something worth reading in a 30-column cell.
pub fn friendly_process(command: &str, cmdline: &str) -> String {
    let parts: Vec<&str> = cmdline.split_whitespace().collect();
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();

    // A process that renamed its own argv[0] — `next-server (v16.3.4)` — is
    // telling us what it is, so believe it over the executable name.
    if let Some(first) = parts.first() {
        let b0 = base(first);
        if !b0.is_empty() && b0 != command && !command.is_empty() {
            return b0;
        }
    }

    // Interpreters carry their identity in argv[1], not argv[0].
    let interpreter = matches!(
        command.to_lowercase().as_str(),
        "node" | "python" | "python3" | "ruby" | "bun" | "deno" | "java" | "php" | "uv"
    );
    if interpreter {
        let useful = parts
            .iter()
            .skip(1)
            .find(|a| !a.starts_with('-') && !a.starts_with('(') && !a.is_empty());
        if let Some(arg) = useful {
            let name = base(arg);
            if !name.is_empty() && name != "-" {
                return format!("{command} {name}");
            }
        }
    }
    if command.is_empty() {
        parts.first().map(|p| base(p)).unwrap_or_default()
    } else {
        command.to_string()
    }
}

/// Port-first classification, with the process name as the tie-breaker.
pub fn classify(command: &str, cmdline: &str, ports: &[u16]) -> Kind {
    let cmd = command.to_lowercase();
    let line = cmdline.to_lowercase();

    for &port in ports {
        let k = match port {
            5432 | 5433 | 3306 | 3307 | 27017 | 27018 | 1433 | 1521 | 5984 | 8529 | 9042
            | 26257 | 7687 | 5439 => Some(Kind::Database),
            6379 | 6380 | 11211 => Some(Kind::Cache),
            9200 | 9300 | 7700 | 8108 | 6333 | 19530 => Some(Kind::Search),
            5672 | 15672 | 9092 | 4222 | 6650 | 61616 => Some(Kind::Queue),
            25 | 465 | 587 | 1025 | 8025 | 143 | 993 => Some(Kind::Mail),
            2375..=2377 => Some(Kind::Container),
            11434 | 8188 | 1234 => Some(Kind::Ai),
            22
            | 53
            | 88
            | 111
            | 123
            | 137..=139
            | 445
            | 548
            | 631
            | 5353
            | 17500
            | 49152..=49250 => Some(Kind::System),
            80 | 443 | 8080 | 8443 => Some(Kind::Proxy),
            3000..=3010 | 4200 | 5173..=5180 | 8000..=8010 | 1313 | 4321 | 5000 | 5001 | 9000 => {
                Some(Kind::Web)
            }
            _ => None,
        };
        if let Some(k) = k {
            // Port ranges are a hint, not a verdict — let obvious process names win.
            if let Some(by_name) = classify_by_name(&cmd, &line) {
                return by_name;
            }
            return k;
        }
    }

    classify_by_name(&cmd, &line).unwrap_or(Kind::Other)
}

/// Process names that give a service away, and what they give away.
///
/// Order is the rule: the first row whose name appears wins, so anything that
/// could belong to two kinds is listed under the more specific one. Written
/// out as a chain of `if`s this was a hundred and thirty lines of branches
/// that only ever did one thing.
/// Process names that give a service away, and what they give away.
///
/// Order is the rule: the first row holding a match wins, so anything that
/// could belong to two kinds is listed under the more specific one. Laid out
/// by hand, because rustfmt gives a hundred and nine names a line each and a
/// table you cannot see the shape of is not a table.
#[rustfmt::skip]
const BY_NAME: &[(Kind, &[&str])] = &[
    (Kind::Database,  &["postgres", "mysqld", "mariadb", "mongod", "cockroach", "clickhouse",
                        "influxd", "surreal", "sqld"]),
    (Kind::Cache,     &["redis", "memcached", "valkey", "dragonfly"]),
    (Kind::Search,    &["elasticsearch", "opensearch", "meilisearch", "typesense", "qdrant",
                        "solr"]),
    (Kind::Queue,     &["rabbitmq", "kafka", "nats-server", "beam.smp", "pulsar"]),
    (Kind::Proxy,     &["nginx", "caddy", "traefik", "envoy", "haproxy", "ngrok", "cloudflared"]),
    (Kind::Container, &["docker", "containerd", "colima", "podman", "lima", "orbstack"]),
    (Kind::Ai,        &["ollama", "llama-server", "vllm", "lmstudio", "comfyui"]),
    (Kind::Mail,      &["mailpit", "mailhog", "postfix", "smtpd"]),
    (Kind::Web,       &["next-server", "next dev", "vite", "webpack", "nuxt", "astro", "remix",
                        "ng serve", "react-scripts", "storybook", "hugo", "jekyll", "rails s"]),
    (Kind::Api,       &["uvicorn", "gunicorn", "fastapi", "hypercorn", "puma", "unicorn",
                        "django", "flask", "nest", "fastify", "express", "actix", "axum",
                        "go run"]),
    (Kind::DevTool,   &["cargo", "turbo", "esbuild", "tsc", "metro", "watchman", "bundler",
                        "air ", "nodemon"]),
    (Kind::System,    &["dropbox", "adobe", "creative cloud", "figma", "spotify", "backblaze",
                        "google drive", "onedrive", "logi", "steam", "zoom", "slack helper"]),
    (Kind::System,    &["rapportd", "sharingd", "controlcenter", "launchd", "mdnsresponder",
                        "cupsd", "sshd", "remoted", "netbiosd", "identityservices", "airplay",
                        "bluetoothd", "distnoted", "nsurlsessiond", "apsd"]),
];

fn classify_by_name(cmd: &str, line: &str) -> Option<Kind> {
    BY_NAME
        .iter()
        .find(|(_, names)| names.iter().any(|n| cmd.contains(n) || line.contains(n)))
        .map(|(kind, _)| *kind)
}

/// Classify, letting user rules win over the built-in tables.
pub fn classify_with(rules: &Rules, command: &str, cmdline: &str, ports: &[u16]) -> Kind {
    rules
        .classify(command, cmdline, ports)
        .unwrap_or_else(|| classify(command, cmdline, ports))
}
