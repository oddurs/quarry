//! Types describing a listening service, plus the heuristics that decide what
//! kind of thing it is. Classification is intentionally port-first: a process
//! called `node` tells you nothing, but `node` on 5432 vs 5173 does.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

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

/// How a service is reachable.
///
/// Not everything listens on a port. A great deal of local software — the
/// Docker daemon, PostgreSQL, PHP-FPM, anything using socket activation — is
/// reachable only through a path on the filesystem, and a tool that claims to
/// show what is running cannot be blind to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub enum Transport {
    Tcp,
    Udp,
    Unix,
}

impl Transport {
    pub fn label(self) -> &'static str {
        match self {
            Transport::Tcp => "tcp",
            Transport::Udp => "udp",
            Transport::Unix => "unix",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    pub transport: Transport,
    pub addr: IpAddr,
    /// Zero for a unix socket, which has no port.
    pub port: u16,
    /// Set for a unix socket, which has no address.
    pub path: Option<PathBuf>,
    /// True when bound to 0.0.0.0 / :: — reachable from the network.
    pub wildcard: bool,
}

impl Listener {
    pub fn tcp(addr: IpAddr, port: u16) -> Listener {
        Listener {
            transport: Transport::Tcp,
            wildcard: addr.is_unspecified(),
            addr,
            port,
            path: None,
        }
    }

    pub fn udp(addr: IpAddr, port: u16) -> Listener {
        Listener {
            transport: Transport::Udp,
            wildcard: addr.is_unspecified(),
            addr,
            port,
            path: None,
        }
    }

    pub fn unix(path: PathBuf) -> Listener {
        Listener {
            transport: Transport::Unix,
            addr: IpAddr::from([0, 0, 0, 0]),
            port: 0,
            path: Some(path),
            wildcard: false,
        }
    }

    pub fn is_unix(&self) -> bool {
        self.transport == Transport::Unix
    }

    /// What to print where a port would go.
    pub fn label(&self) -> String {
        match &self.path {
            Some(path) => shorten_path(path),
            None => self.port.to_string(),
        }
    }

    pub fn scope(&self) -> &'static str {
        match self.transport {
            Transport::Unix => "filesystem",
            _ if self.wildcard => "all interfaces",
            _ if self.addr.is_loopback() => "loopback",
            _ => "interface",
        }
    }

    /// A UDP socket is bound, not listening; nothing can be tested by
    /// connecting to it, and saying "open" would imply a check that did not
    /// happen.
    pub fn is_connectable(&self) -> bool {
        self.transport != Transport::Udp
    }
}

/// The tail of a socket path, which is the part that identifies it. A full
/// path is longer than the column and its interesting end is on the right.
pub fn shorten_path(path: &std::path::Path) -> String {
    let full = path.to_string_lossy();
    match path.file_name() {
        Some(name) if full.len() > 28 => name.to_string_lossy().to_string(),
        _ => full.to_string(),
    }
}

/// Where a group's name came from. Drives both ordering and styling: a real
/// repository is a stronger claim than a directory that merely has a name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GroupSource {
    Repo,
    Folder,
    Unattributed,
    System,
}

impl GroupSource {
    pub fn rank(self) -> u8 {
        match self {
            GroupSource::Repo => 0,
            GroupSource::Folder => 1,
            GroupSource::Unattributed => 2,
            GroupSource::System => 3,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Repo {
    pub name: String,
    pub root: PathBuf,
    pub branch: Option<String>,
    pub remote: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub enum Health {
    #[default]
    Unknown,
    /// Answered an HTTP request.
    Http {
        status: u16,
        scheme: &'static str,
        latency: Duration,
        server: Option<String>,
        title: Option<String>,
        is_html: bool,
    },
    /// Accepted a TCP connection but is not HTTP (or refused to answer one).
    Open { latency: Duration },
    /// Bound, but nothing was tested. A UDP socket cannot be connected to, and
    /// saying "open" would claim a check that did not happen.
    Bound,
    /// The port is gone since the last scan.
    Closed,
}

impl Health {
    /// The glyph alone, so a colour-blind reader and a `NO_COLOR` terminal get
    /// the same information as everyone else.
    pub fn glyph(&self) -> &'static str {
        match self {
            Health::Unknown => "○",
            Health::Bound => "◍",
            Health::Open { .. } => "●",
            Health::Closed => "✕",
            Health::Http { status, .. } => match status {
                // Answered, and asked who you are. Nothing is wrong: the
                // service is running and doing exactly what it should.
                401 | 403 => "◆",
                400..=599 => "▲",
                _ => "●",
            },
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Health::Unknown => "checking".into(),
            Health::Bound => "bound".into(),
            Health::Open { latency } => format!("open · {}", fmt_ms(*latency)),
            Health::Closed => "not responding".into(),
            Health::Http {
                status, latency, ..
            } if self.is_protected() => {
                format!("{status} protected · {}", fmt_ms(*latency))
            }
            Health::Http {
                status, latency, ..
            } => {
                format!("{} {} · {}", status, status_text(*status), fmt_ms(*latency))
            }
        }
    }

    /// Rank for sorting: healthy things first, broken things last.
    pub fn rank(&self) -> u8 {
        match self {
            Health::Http { status, .. } if *status < 400 => 0,
            // A service asking who you are is working, so it sorts with the
            // working ones rather than with the failures.
            _ if self.is_protected() => 1,
            Health::Http { .. } => 2,
            Health::Open { .. } => 3,
            Health::Bound => 4,
            Health::Unknown => 5,
            Health::Closed => 6,
        }
    }

    /// Answered, and asked for credentials. Anything behind auth — Grafana,
    /// Keycloak, a private API, an admin panel — is healthy, and colouring it
    /// like a 404 teaches you to ignore the colour.
    pub fn is_protected(&self) -> bool {
        matches!(
            self,
            Health::Http {
                status: 401 | 403,
                ..
            }
        )
    }

    pub fn is_trouble(&self) -> bool {
        matches!(self, Health::Closed)
            || matches!(self, Health::Http { status, .. } if *status >= 500)
    }
}

pub fn fmt_ms(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 10.0 {
        format!("{ms:.1}ms")
    } else {
        format!("{ms:.0}ms")
    }
}

pub fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved",
        302 => "Found",
        304 => "Not Modified",
        307 | 308 => "Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Not Allowed",
        426 => "Upgrade Required",
        500 => "Server Error",
        502 => "Bad Gateway",
        503 => "Unavailable",
        _ => "",
    }
}

#[derive(Clone, Debug)]
pub struct Server {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub command: String,
    pub cmdline: String,
    pub exe: Option<PathBuf>,
    pub user: String,
    pub cwd: Option<PathBuf>,
    pub listeners: Vec<Listener>,
    pub repo: Option<Repo>,
    pub kind: Kind,
    /// What the signature table decided this is — "PostgreSQL", "Next.js" — as
    /// distinct from the process that happens to be running it.
    pub service: Option<String>,
    /// The path that means healthy, from the signature.
    pub health_path: Option<String>,
    /// What a user would paste into a client, from the signature.
    pub uri: Option<String>,
    /// Anything the signature wanted to say about this service.
    pub note: Option<String>,
    /// A named handshake from the signature, for a protocol that will not speak
    /// until spoken to.
    pub handshake: Option<String>,
    /// What the service said on connect, if anything.
    pub banner: Option<Vec<u8>>,
    /// Why it was identified as it was, in the words the signature table used.
    pub evidence: Vec<String>,
    /// The container behind this port, where the runtime published one.
    pub container: Option<crate::docker::Container>,
    pub started_at: u64,
    pub cpu: f32,
    pub mem: u64,
    pub health: Health,
}

impl Server {
    /// The listener we treat as the service's front door. Sorted so that a TCP
    /// port comes before a unix socket: a service reachable both ways is
    /// usually reached over the port.
    pub fn primary(&self) -> Option<&Listener> {
        self.listeners.first()
    }

    pub fn primary_port(&self) -> u16 {
        self.listeners.first().map(|l| l.port).unwrap_or(0)
    }

    /// What to show where a port would go.
    pub fn primary_label(&self) -> String {
        self.primary().map(|l| l.label()).unwrap_or_default()
    }

    /// True when nothing here can be reached over a port.
    pub fn is_socket_only(&self) -> bool {
        self.listeners.iter().all(|l| l.is_unix())
    }

    pub fn scheme(&self) -> &'static str {
        if let Health::Http { scheme, .. } = &self.health {
            return scheme;
        }
        scheme_for(self.primary_port())
    }

    pub fn url(&self) -> String {
        if let Some(uri) = &self.uri {
            return uri.clone();
        }
        match self.primary() {
            Some(l) if l.is_unix() => format!(
                "unix:{}",
                l.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
            _ => format!("{}://localhost:{}", self.scheme(), self.primary_port()),
        }
    }

    /// What to call this service in a list: the name the signature gave it, or
    /// failing that the process, tidied.
    pub fn service_name(&self) -> String {
        // The container first: the daemon knows what is behind the port, and a
        // signature can only guess from the host process that published it —
        // which is the runtime, and says nothing about the service.
        if let Some(container) = &self.container {
            return container.display_name().to_string();
        }
        if let Some(name) = &self.service {
            return name.clone();
        }
        friendly_process(&self.command, &self.cmdline)
    }

    /// The label a row leads with — the repo name if we found one, else the
    /// process, trimmed of the paths that make `node` rows unreadable.
    pub fn title(&self) -> String {
        if let Some(repo) = &self.repo {
            return repo.name.clone();
        }
        if let Some(project) = self.container.as_ref().and_then(|c| c.project.as_deref()) {
            return project.to_string();
        }
        if let Some(folder) = self.folder_name() {
            return folder;
        }
        friendly_process(&self.command, &self.cmdline)
    }

    /// The project a service belongs to: its repository if it has one, else the
    /// directory it is running in. A process started in a plain folder is still
    /// a project to whoever started it.
    pub fn group_key(&self) -> String {
        if let Some(r) = &self.repo {
            return r.name.clone();
        }
        // A Compose project is a project, whether or not its directory
        // resolved to a repository.
        if let Some(project) = self.container.as_ref().and_then(|c| c.project.as_deref()) {
            return project.to_string();
        }
        if let Some(folder) = self.folder_name() {
            return folder;
        }
        if self.kind == Kind::System {
            "system".into()
        } else {
            "unattributed".into()
        }
    }

    pub fn group_source(&self) -> GroupSource {
        if self.repo.is_some() {
            GroupSource::Repo
        } else if self.container.as_ref().is_some_and(|c| c.project.is_some())
            || self.folder_name().is_some()
        {
            // A Compose project and a directory are the same kind of claim: a
            // name that came from somewhere real, but not from a repository.
            GroupSource::Folder
        } else if self.kind == Kind::System {
            GroupSource::System
        } else {
            GroupSource::Unattributed
        }
    }

    /// The working directory's name, when it is specific enough to mean
    /// something. `/`, `$HOME` and the temp directory name nothing in
    /// particular, so they are not worth grouping by.
    pub fn folder_name(&self) -> Option<String> {
        let cwd = self.cwd.as_ref()?;
        if is_anonymous_dir(cwd) {
            return None;
        }
        let name = cwd.file_name()?.to_string_lossy().to_string();
        (!name.is_empty()).then_some(name)
    }

    pub fn uptime(&self, now: u64) -> Option<Duration> {
        now.checked_sub(self.started_at).map(Duration::from_secs)
    }

    /// Whether a filter matches. `needle` must already be lowercase — the
    /// caller lowercases once rather than every service lowercasing it again.
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        // Searching by project name has to work whether that name came from a
        // repository or from a directory. Checked in rough order of how often
        // it is what the user meant, and how cheap it is to look at.
        if self.listeners.iter().any(|l| port_contains(l.port, needle)) {
            return true;
        }
        if contains_ci(self.kind.label(), needle) || contains_ci(&self.command, needle) {
            return true;
        }
        if let Some(service) = &self.service
            && contains_ci(service, needle)
        {
            return true;
        }
        if let Some(repo) = &self.repo {
            if contains_ci(&repo.name, needle) {
                return true;
            }
        } else if let Some(folder) = self.folder_name()
            && contains_ci(&folder, needle)
        {
            return true;
        }
        contains_ci(&self.cmdline, needle)
    }
}

/// Whether a port's decimal form contains `needle`, without rendering it.
/// Five digits fit in a stack buffer; a filter runs this once per listener per
/// keystroke.
fn port_contains(port: u16, needle: &str) -> bool {
    let mut buf = [0u8; 5];
    let mut n = port;
    let mut len = 0;
    loop {
        buf[4 - len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
        if n == 0 {
            break;
        }
    }
    let digits = &buf[5 - len..];
    // SAFETY-free: the bytes written above are ASCII digits by construction.
    let text = std::str::from_utf8(digits).unwrap_or("");
    text.contains(needle)
}

/// `haystack.to_lowercase().contains(needle)` without the allocation.
///
/// Filtering runs over every service on every keystroke, and allocating two
/// lowercase copies per service per character is most of what that used to
/// cost. ASCII takes the fast path, which is every port number and nearly every
/// process name; anything else falls back to the correct-but-slower form.
pub fn contains_ci(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if haystack.len() < needle_lower.len() {
        return false;
    }
    if haystack.is_ascii() && needle_lower.is_ascii() {
        let hay = haystack.as_bytes();
        let needle = needle_lower.as_bytes();
        return hay.windows(needle.len()).any(|w| {
            w.iter()
                .zip(needle)
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
        });
    }
    haystack.to_lowercase().contains(needle_lower)
}

/// Directories that name no particular project.
fn is_anonymous_dir(path: &std::path::Path) -> bool {
    if path.parent().is_none() {
        return true; // the filesystem root
    }
    let anonymous = ["/", "/tmp", "/var", "/usr", "/private/tmp", "/var/root"];
    if anonymous.iter().any(|a| path == std::path::Path::new(a)) {
        return true;
    }
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => path == std::path::Path::new(&home),
        _ => false,
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

fn classify_by_name(cmd: &str, line: &str) -> Option<Kind> {
    let has = |needle: &str| cmd.contains(needle) || line.contains(needle);

    if has("postgres")
        || has("mysqld")
        || has("mariadb")
        || has("mongod")
        || has("cockroach")
        || has("clickhouse")
        || has("influxd")
        || has("surreal")
        || has("sqld")
    {
        return Some(Kind::Database);
    }
    if has("redis") || has("memcached") || has("valkey") || has("dragonfly") {
        return Some(Kind::Cache);
    }
    if has("elasticsearch")
        || has("opensearch")
        || has("meilisearch")
        || has("typesense")
        || has("qdrant")
        || has("solr")
    {
        return Some(Kind::Search);
    }
    if has("rabbitmq") || has("kafka") || has("nats-server") || has("beam.smp") || has("pulsar") {
        return Some(Kind::Queue);
    }
    if has("nginx")
        || has("caddy")
        || has("traefik")
        || has("envoy")
        || has("haproxy")
        || has("ngrok")
        || has("cloudflared")
    {
        return Some(Kind::Proxy);
    }
    if has("docker")
        || has("containerd")
        || has("colima")
        || has("podman")
        || has("lima")
        || has("orbstack")
    {
        return Some(Kind::Container);
    }
    if has("ollama") || has("llama-server") || has("vllm") || has("lmstudio") || has("comfyui") {
        return Some(Kind::Ai);
    }
    if has("mailpit") || has("mailhog") || has("postfix") || has("smtpd") {
        return Some(Kind::Mail);
    }
    if has("next-server")
        || has("next dev")
        || has("vite")
        || has("webpack")
        || has("nuxt")
        || has("astro")
        || has("remix")
        || has("ng serve")
        || has("react-scripts")
        || has("storybook")
        || has("hugo")
        || has("jekyll")
        || has("rails s")
    {
        return Some(Kind::Web);
    }
    if has("uvicorn")
        || has("gunicorn")
        || has("fastapi")
        || has("hypercorn")
        || has("puma")
        || has("unicorn")
        || has("django")
        || has("flask")
        || has("nest")
        || has("fastify")
        || has("express")
        || has("actix")
        || has("axum")
        || has("go run")
    {
        return Some(Kind::Api);
    }
    if has("cargo")
        || has("turbo")
        || has("esbuild")
        || has("tsc")
        || has("metro")
        || has("watchman")
        || has("bundler")
        || has("air ")
        || has("nodemon")
    {
        return Some(Kind::DevTool);
    }
    if has("dropbox")
        || has("adobe")
        || has("creative cloud")
        || has("figma")
        || has("spotify")
        || has("backblaze")
        || has("google drive")
        || has("onedrive")
        || has("logi")
        || has("steam")
        || has("zoom")
        || has("slack helper")
    {
        return Some(Kind::System);
    }
    if has("rapportd")
        || has("sharingd")
        || has("controlcenter")
        || has("launchd")
        || has("mdnsresponder")
        || has("cupsd")
        || has("sshd")
        || has("remoted")
        || has("netbiosd")
        || has("identityservices")
        || has("airplay")
        || has("bluetoothd")
        || has("distnoted")
        || has("nsurlsessiond")
        || has("apsd")
    {
        return Some(Kind::System);
    }
    None
}

/// Classification rules a user has added, merged over the built-in tables.
///
/// The built-in guesses are good about common software and wrong about anybody's
/// private service on port 9174. These let that be fixed in a config file
/// instead of a pull request, and user rules win: a rule that never overrides
/// anything is not a rule.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    ports: std::collections::BTreeMap<u16, Kind>,
    names: Vec<(String, Kind)>,
}

impl Rules {
    /// Build from the config, returning what could not be understood so the
    /// caller can report it. A bad rule is skipped, never fatal.
    pub fn from_config(
        ports: &std::collections::BTreeMap<String, String>,
        names: &std::collections::BTreeMap<String, String>,
    ) -> (Rules, Vec<String>) {
        let mut rules = Rules::default();
        let mut problems = Vec::new();

        for (port, kind) in ports {
            match (port.trim().parse::<u16>(), Kind::from_label(kind)) {
                (Ok(p), Some(k)) if p != 0 => {
                    rules.ports.insert(p, k);
                }
                (Err(_), _) => problems.push(format!("[ports] {port:?} is not a port number")),
                (_, None) => problems.push(format!("[ports] {port} = {kind:?} is not a kind")),
                _ => problems.push(format!("[ports] {port:?} is not a port number")),
            }
        }
        for (pattern, kind) in names {
            match Kind::from_label(kind) {
                Some(k) => rules.names.push((pattern.to_lowercase(), k)),
                None => problems.push(format!("[names] {pattern:?} = {kind:?} is not a kind")),
            }
        }
        // Longest pattern first, so a specific rule beats a broad one.
        rules
            .names
            .sort_by_key(|(p, _)| std::cmp::Reverse(p.chars().filter(|c| *c != '*').count()));

        (rules, problems)
    }

    pub fn is_empty(&self) -> bool {
        self.ports.is_empty() && self.names.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ports.len() + self.names.len()
    }

    /// A user rule, if one matches. Ports are checked before names, matching
    /// the built-in order.
    pub fn classify(&self, command: &str, cmdline: &str, ports: &[u16]) -> Option<Kind> {
        for port in ports {
            if let Some(kind) = self.ports.get(port) {
                return Some(*kind);
            }
        }
        let command = command.to_lowercase();
        let cmdline = cmdline.to_lowercase();
        for (pattern, kind) in &self.names {
            if glob_match(pattern, &command) || glob_match(pattern, &cmdline) {
                return Some(*kind);
            }
        }
        None
    }
}

/// Classify, letting user rules win over the built-in tables.
pub fn classify_with(rules: &Rules, command: &str, cmdline: &str, ports: &[u16]) -> Kind {
    rules
        .classify(command, cmdline, ports)
        .unwrap_or_else(|| classify(command, cmdline, ports))
}

/// Match a process pattern against a command line.
///
/// A literal pattern must land on a word boundary. Plain substring matching is
/// far too loose for process names, and the failures are not theoretical: on
/// this machine `serve` matched `redis-server`, and `dex` matched `index.ts`,
/// so quarry announced a static file server and an OIDC provider that were not
/// there. Short names are common and argv is full of paths.
///
/// A pattern containing `*` is a glob and means what it says.
pub fn process_match(pattern: &str, text: &str) -> bool {
    if pattern.contains('*') {
        return glob_match(pattern, text);
    }
    if pattern.is_empty() || pattern.len() > text.len() {
        return false;
    }
    let bytes = text.as_bytes();
    let needle = pattern.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric();

    let mut from = 0;
    while let Some(at) = text[from..].find(pattern).map(|i| i + from) {
        let before_ok = at == 0 || !word(bytes[at - 1]);
        let after = at + needle.len();
        let after_ok = after >= bytes.len() || !word(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        from = at + 1;
        if from >= text.len() {
            break;
        }
    }
    false
}

/// `*` matches any run of characters, anywhere. Deliberately not a full glob:
/// argv is messy enough without a regex dialect in the config file.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if !pattern.contains('*') {
        return text.contains(pattern);
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        match rest.find(part) {
            Some(at) => {
                // A pattern that does not start with `*` must match at the start.
                if i == 0 && !pattern.starts_with('*') && at != 0 {
                    return false;
                }
                rest = &rest[at + part.len()..];
            }
            None => return false,
        }
    }
    // A pattern that does not end with `*` must reach the end.
    if !pattern.ends_with('*')
        && let Some(last) = parts.last().filter(|p| !p.is_empty())
    {
        return text.ends_with(last);
    }
    true
}

#[cfg(test)]
mod rule_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn rules(ports: &[(&str, &str)], names: &[(&str, &str)]) -> (Rules, Vec<String>) {
        let p: BTreeMap<String, String> = ports
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let n: BTreeMap<String, String> = names
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        Rules::from_config(&p, &n)
    }

    #[test]
    fn a_user_port_rule_wins_over_the_built_in_table() {
        // 3000 is a web port as far as the built-in table is concerned.
        assert_eq!(classify("node", "", &[3000]), Kind::Web);
        let (r, problems) = rules(&[("3000", "queue")], &[]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(classify_with(&r, "node", "", &[3000]), Kind::Queue);
    }

    #[test]
    fn a_port_quarry_has_never_heard_of_can_be_named() {
        assert_eq!(classify("mine", "", &[9174]), Kind::Other);
        let (r, _) = rules(&[("9174", "queue")], &[]);
        assert_eq!(classify_with(&r, "mine", "", &[9174]), Kind::Queue);
    }

    #[test]
    fn name_rules_glob() {
        let (r, problems) = rules(&[], &[("*-worker", "queue"), ("my-daemon", "api")]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            classify_with(&r, "billing-worker", "", &[8123]),
            Kind::Queue
        );
        assert_eq!(classify_with(&r, "my-daemon", "", &[8123]), Kind::Api);
        assert_eq!(classify_with(&r, "unrelated", "", &[8123]), Kind::Other);
    }

    #[test]
    fn a_bad_rule_is_reported_and_skipped_rather_than_fatal() {
        let (r, problems) = rules(&[("not-a-port", "web"), ("80", "not-a-kind")], &[]);
        assert!(r.is_empty(), "neither rule should have survived");
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("not a port number")));
        assert!(problems.iter().any(|p| p.contains("not a kind")));
    }

    /// The failures that produced this rule, taken from a real machine.
    #[test]
    fn a_short_process_name_does_not_match_the_middle_of_a_word() {
        assert!(
            !process_match("serve", "redis-server *:6379"),
            "`serve` inside `redis-server` announced a static file server"
        );
        assert!(
            !process_match("dex", "node index.ts"),
            "`dex` inside `index.ts` announced an OIDC provider"
        );
        assert!(!process_match("api", "rapidapi-thing"));

        // And the matches that must keep working.
        assert!(process_match("serve", "node /app/node_modules/.bin/serve"));
        assert!(process_match("serve", "serve -s build"));
        assert!(process_match(
            "postgres",
            "/usr/local/bin/postgres -D /data"
        ));
        assert!(process_match("next-server", "next-server (v16.3.4)"));
        assert!(process_match("redis-server", "redis-server *:6379"));
        assert!(process_match("node", "node index.ts"));
        // A glob still means what it says.
        assert!(process_match("*-worker", "billing-worker"));
    }

    #[test]
    fn globs_match_where_you_would_expect() {
        assert!(glob_match("worker", "billing-worker"));
        assert!(glob_match("*-worker", "billing-worker"));
        assert!(glob_match("billing-*", "billing-worker"));
        assert!(glob_match("*ill*ork*", "billing-worker"));
        assert!(!glob_match("*-worker", "worker-billing"));
        assert!(!glob_match("nope", "billing-worker"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn every_kind_label_round_trips() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_label(kind.label()), Some(kind), "{kind:?}");
        }
        assert_eq!(Kind::from_label("nonsense"), None);
    }
}

#[cfg(test)]
mod health_tests {
    use super::*;

    /// Anything behind auth — Grafana, Keycloak, an admin panel — is running
    /// exactly as intended. Drawing it like a 404 teaches you to ignore the
    /// colour, which is the one thing the colour must never do.
    #[test]
    fn a_protected_service_is_not_a_broken_one() {
        let protected = Health::Http {
            status: 401,
            scheme: "http",
            latency: Duration::from_millis(4),
            server: None,
            title: None,
            is_html: false,
        };
        assert!(protected.is_protected());
        assert!(!protected.is_trouble(), "401 is not a failure");
        assert!(
            protected.rank()
                < Health::Open {
                    latency: Duration::ZERO
                }
                .rank(),
            "it sorts with the working services"
        );
        assert!(
            protected.summary().contains("protected"),
            "{}",
            protected.summary()
        );

        // And the glyph differs, so the distinction survives `mono` and a
        // colour-blind reader.
        let missing = Health::Http {
            status: 404,
            scheme: "http",
            latency: Duration::from_millis(4),
            server: None,
            title: None,
            is_html: false,
        };
        assert_ne!(protected.glyph(), missing.glyph());
        assert!(!missing.is_protected());
    }

    #[test]
    fn every_health_state_has_its_own_glyph_where_it_matters() {
        let states = [
            Health::Unknown,
            Health::Bound,
            Health::Closed,
            Health::Open {
                latency: Duration::ZERO,
            },
        ];
        let mut glyphs: Vec<&str> = states.iter().map(|h| h.glyph()).collect();
        glyphs.sort_unstable();
        let before = glyphs.len();
        glyphs.dedup();
        assert_eq!(before, glyphs.len(), "two states share a glyph: {glyphs:?}");
    }

    #[test]
    fn ranking_puts_working_before_broken() {
        let ok = Health::Http {
            status: 200,
            scheme: "http",
            latency: Duration::ZERO,
            server: None,
            title: None,
            is_html: true,
        };
        assert!(ok.rank() < Health::Closed.rank());
        assert!(
            Health::Open {
                latency: Duration::ZERO
            }
            .rank()
                < Health::Closed.rank()
        );
        assert!(Health::Bound.rank() < Health::Closed.rank());
    }
}
