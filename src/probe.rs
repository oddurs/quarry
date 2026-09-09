//! Health checks.
//!
//! Every listener gets a TCP connect; anything that might speak HTTP also gets
//! a `GET /`. Probes run on a fixed pool so a hung service delays only itself,
//! and the pool is bounded so a machine with a thousand listeners cannot spawn
//! a thousand threads.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::handshake;
use crate::model::{Health, Kind, scheme_for};

pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
pub const HTTP_TIMEOUT: Duration = Duration::from_millis(1800);
/// How long to wait for a service to introduce itself.
///
/// Measured rather than chosen: a server that greets on accept is heard within
/// a millisecond, and the window costs nothing when a banner is coming because
/// the read returns the moment bytes arrive. It is paid in full only by silent
/// services — which is why this runs in a second pass, over the few services
/// the first pass could not identify. OrbStack's sshd, behind a VM boundary,
/// needed about 50ms; 250 leaves room for something slower.
pub const BANNER_WINDOW: Duration = Duration::from_millis(250);
pub const WORKERS: usize = 12;
/// Beyond this many queued probes we start dropping, rather than building a
/// backlog the user will never see the results of.
pub const MAX_QUEUE: usize = 4096;
pub const MAX_BODY: u64 = 96 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub pid: u32,
    pub port: u16,
    pub addr: IpAddr,
    pub transport: crate::model::Transport,
    /// Set for a unix socket, which is dialled by path rather than by address.
    pub socket_path: Option<std::path::PathBuf>,
    pub kind: Kind,
    /// The path to request. A dev server that 404s on `/` reads as unhealthy
    /// while being perfectly fine, which trains you to ignore the colour.
    pub path: String,
    /// A named handshake from the signature table, for protocols that will not
    /// speak until spoken to.
    pub handshake: Option<String>,
}

impl Target {
    /// A target that asks for the site root, which is what most things want.
    pub fn root(pid: u32, port: u16, addr: IpAddr, kind: Kind) -> Target {
        Target {
            pid,
            port,
            addr,
            kind,
            transport: crate::model::Transport::Tcp,
            socket_path: None,
            path: "/".to_string(),
            handshake: None,
        }
    }

    /// A target from a discovered listener.
    pub fn from_listener(pid: u32, listener: &crate::model::Listener, kind: Kind) -> Target {
        Target {
            pid,
            port: listener.port,
            addr: listener.addr,
            transport: listener.transport,
            socket_path: listener.path.clone(),
            kind,
            path: "/".to_string(),
            handshake: None,
        }
    }
}

impl Target {
    /// A wildcard bind is reachable on loopback; anything else we dial directly.
    pub fn socket(&self) -> SocketAddr {
        let ip = if self.addr.is_unspecified() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            self.addr
        };
        SocketAddr::new(ip, self.port)
    }
}

#[derive(Debug)]
pub struct Outcome {
    pub pid: u32,
    pub port: u16,
    pub health: Health,
    /// What the service said, unprompted, if anything. Evidence for a second
    /// pass at identification.
    pub banner: Option<Vec<u8>>,
}

/// Everything one probe learned.
#[derive(Debug, Clone, Default)]
pub struct Observation {
    pub health: Health,
    pub banner: Option<Vec<u8>>,
}

impl From<Health> for Observation {
    fn from(health: Health) -> Self {
        Observation {
            health,
            banner: None,
        }
    }
}

/// How a target gets checked. The real one talks to the network; tests use a
/// scripted one so a health-dependent assertion never depends on a live port.
pub trait Prober: Send + Sync + 'static {
    fn probe(&self, target: &Target) -> Observation;
}

pub struct NetProber {
    connect_timeout: Duration,
    banner_window: Duration,
    max_body: u64,
    /// Built once and shared. Constructing an agent means building a TLS
    /// configuration and a connection pool; doing that per probe threw both
    /// away on every request, on every service, every six seconds.
    agent: ureq::Agent,
}

impl NetProber {
    pub fn new() -> Self {
        Self::with_timeouts(CONNECT_TIMEOUT, HTTP_TIMEOUT)
    }

    pub fn with_timeouts(connect: Duration, http: Duration) -> Self {
        Self {
            connect_timeout: connect,
            banner_window: BANNER_WINDOW,
            max_body: MAX_BODY,
            // The request timeout lives in the agent.
            agent: build_agent(http),
        }
    }

    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            connect_timeout: config.connect_timeout(),
            banner_window: Duration::from_millis(config.banner_ms),
            max_body: config.max_body(),
            agent: build_agent(config.request_timeout()),
        }
    }
}

impl Default for NetProber {
    fn default() -> Self {
        Self::new()
    }
}

impl Prober for NetProber {
    fn probe(&self, target: &Target) -> Observation {
        use crate::model::Transport;

        match target.transport {
            // Nothing can be tested by connecting to a UDP socket. Reporting it
            // as open would claim a check that did not happen.
            Transport::Udp => return Health::Bound.into(),
            Transport::Unix => return self.probe_unix(target),
            Transport::Tcp => {}
        }

        let sock = target.socket();

        // Something we know does not speak HTTP goes straight to listening and
        // then to whatever handshake its signature names.
        if !target.kind.speaks_http() {
            return self.listen_then_ask(target, sock);
        }

        let host = match sock.ip() {
            IpAddr::V6(ip) => format!("[{ip}]"),
            IpAddr::V4(ip) => ip.to_string(),
        };
        let first = scheme_for(target.port);

        // Try HTTP before anything else: on the common path that is one
        // connection instead of two, and a refused connection fails just as
        // fast through ureq as it does through a raw connect.
        let started = Instant::now();
        if let Some(h) = http_probe(
            &self.agent,
            first,
            &host,
            target.port,
            &target.path,
            self.max_body,
        ) {
            return h.into();
        }
        let first_attempt = started.elapsed();

        // It did not answer HTTP. Is it even there?
        let started = Instant::now();
        let Ok(_) = TcpStream::connect_timeout(&sock, self.connect_timeout) else {
            return Health::Closed.into();
        };
        let tcp_latency = started.elapsed();

        // A fast failure means a protocol mismatch, which is worth one retry on
        // the other scheme. A slow one means a socket that accepts and never
        // answers, and retrying would only cost another full timeout.
        if first_attempt < Duration::from_millis(500) {
            let second = if first == "https" { "http" } else { "https" };
            if let Some(h) = http_probe(
                &self.agent,
                second,
                &host,
                target.port,
                &target.path,
                self.max_body,
            ) {
                return h.into();
            }
        }

        // Second pass, for the few services the first could not identify: give
        // them a chance to introduce themselves, and ask directly if a
        // handshake is known for them.
        let mut observation = self.listen_then_ask(target, sock);
        if matches!(observation.health, Health::Open { .. }) {
            observation.health = Health::Open {
                latency: tcp_latency,
            };
        }
        observation
    }
}

impl NetProber {
    /// A unix socket is dialled by path. Everything else about it is the same:
    /// connect, listen for a greeting, ask if we know how.
    fn probe_unix(&self, target: &Target) -> Observation {
        use std::os::unix::net::UnixStream;

        let Some(path) = &target.socket_path else {
            return Health::Closed.into();
        };
        let started = Instant::now();
        let Ok(stream) = UnixStream::connect(path) else {
            // A socket file whose server has gone leaves the path behind, so
            // this is the common and correct answer for a stale one.
            return Health::Closed.into();
        };
        let latency = started.elapsed();

        if !self.banner_window.is_zero() {
            let _ = stream.set_read_timeout(Some(self.banner_window));
            let mut buf = [0u8; 256];
            if let Ok(n) = (&stream).read(&mut buf)
                && n > 0
            {
                return Observation {
                    health: Health::Open { latency },
                    banner: Some(buf[..n].to_vec()),
                };
            }
        }
        Health::Open { latency }.into()
    }

    /// Connect, wait briefly for the service to speak first, and then ask it
    /// directly if we know how.
    fn listen_then_ask(&self, target: &Target, sock: SocketAddr) -> Observation {
        let started = Instant::now();
        let Ok(stream) = TcpStream::connect_timeout(&sock, self.connect_timeout) else {
            return Health::Closed.into();
        };
        let latency = started.elapsed();

        let banner = read_banner(&stream, self.banner_window);
        if banner.is_some() {
            return Observation {
                health: Health::Open { latency },
                banner,
            };
        }

        // Silent. If the signature named a handshake, this is where it is worth
        // spending a round trip.
        if let Some(name) = &target.handshake
            && let Some(reply) = handshake::run(name, &stream, self.banner_window)
        {
            return Observation {
                health: Health::Open { latency },
                banner: Some(reply),
            };
        }
        Observation {
            health: Health::Open { latency },
            banner: None,
        }
    }
}

/// Read whatever a service volunteers, up to a short deadline.
///
/// Nothing is written first. A protocol that expects the client to speak simply
/// stays silent and costs the window; a protocol that greets is heard at once.
fn read_banner(stream: &TcpStream, window: Duration) -> Option<Vec<u8>> {
    if window.is_zero() {
        return None;
    }
    let mut stream = stream.try_clone().ok()?;
    stream.set_read_timeout(Some(window)).ok()?;
    let mut buf = [0u8; 256];
    match stream.read(&mut buf) {
        Ok(0) => None,
        Ok(n) => Some(buf[..n].to_vec()),
        Err(_) => None,
    }
}

/// A prober with predetermined answers, keyed by port.
pub struct ScriptedProber {
    answers: std::collections::HashMap<u16, Health>,
    default: Health,
    pub calls: Arc<AtomicUsize>,
    delay: Duration,
}

impl ScriptedProber {
    pub fn new(answers: std::collections::HashMap<u16, Health>) -> Self {
        Self {
            answers,
            default: Health::Closed,
            calls: Arc::new(AtomicUsize::new(0)),
            delay: Duration::ZERO,
        }
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

impl Prober for ScriptedProber {
    fn probe(&self, target: &Target) -> Observation {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        self.answers
            .get(&target.port)
            .cloned()
            .unwrap_or_else(|| self.default.clone())
            .into()
    }
}

/// A fixed pool that outlives individual scans, so we are not spawning a dozen
/// threads every refresh. Dropping the pool joins its workers.
pub struct Pool {
    tx: Option<Sender<Target>>,
    rx: Receiver<Outcome>,
    queued: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl Pool {
    pub fn new(prober: Arc<dyn Prober>) -> Self {
        Self::with_workers(prober, WORKERS)
    }

    pub fn with_workers(prober: Arc<dyn Prober>, workers: usize) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Target>();
        let (out_tx, out_rx) = mpsc::channel::<Outcome>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let queued = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers.max(1) {
            let jobs = Arc::clone(&job_rx);
            let out = out_tx.clone();
            let prober = Arc::clone(&prober);
            let queued = Arc::clone(&queued);
            handles.push(std::thread::spawn(move || {
                loop {
                    // Hold the lock only long enough to take one job, so a slow
                    // probe never blocks the other workers from picking up work.
                    let target = {
                        let Ok(guard) = jobs.lock() else { return };
                        match guard.recv() {
                            Ok(t) => t,
                            Err(_) => return,
                        }
                    };
                    queued.fetch_sub(1, Ordering::Relaxed);
                    let observed = prober.probe(&target);
                    let outcome = Outcome {
                        pid: target.pid,
                        port: target.port,
                        health: observed.health,
                        banner: observed.banner,
                    };
                    if out.send(outcome).is_err() {
                        return;
                    }
                }
            }));
        }

        Self {
            tx: Some(job_tx),
            rx: out_rx,
            queued,
            dropped: Arc::new(AtomicUsize::new(0)),
            workers: handles,
        }
    }

    /// Queue a probe. Returns false if the queue is saturated and the target was
    /// dropped — the next scan will submit it again.
    pub fn submit(&self, target: Target) -> bool {
        if self.queued.load(Ordering::Relaxed) >= MAX_QUEUE {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let Some(tx) = self.tx.as_ref() else {
            return false;
        };
        self.queued.fetch_add(1, Ordering::Relaxed);
        if tx.send(target).is_err() {
            self.queued.fetch_sub(1, Ordering::Relaxed);
            return false;
        }
        true
    }

    pub fn drain(&self) -> Vec<Outcome> {
        self.rx.try_iter().collect()
    }

    /// Block until `n` outcomes have arrived or the deadline passes. Only used
    /// by the one-shot path and by tests.
    pub fn collect(&self, n: usize, deadline: Duration) -> Vec<Outcome> {
        let end = Instant::now() + deadline;
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match self.rx.recv_timeout(left) {
                Ok(o) => out.push(o),
                Err(_) => break,
            }
        }
        out
    }

    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // Closing the job channel is what tells the workers to stop.
        self.tx.take();
        for h in self.workers.drain(..) {
            let _ = h.join();
        }
    }
}

fn build_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent("quarry")
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .disable_verification(true)
                .build(),
        )
        .build()
        .into()
}

fn http_probe(
    agent: &ureq::Agent,
    scheme: &'static str,
    host: &str,
    port: u16,
    path: &str,
    max_body: u64,
) -> Option<Health> {
    let path = if path.starts_with('/') { path } else { "/" };
    let url = format!("{scheme}://{host}:{port}{path}");

    let started = Instant::now();
    let mut resp = agent.get(&url).call().ok()?;
    let latency = started.elapsed();

    let status = resp.status().as_u16();
    let header = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    };
    let server = header("server");
    let content_type = header("content-type").unwrap_or_default();

    let is_html = content_type.contains("html");
    let title = if is_html {
        let mut body = String::new();
        let _ = resp
            .body_mut()
            .as_reader()
            .take(max_body)
            .read_to_string(&mut body);
        extract_title(&body)
    } else {
        None
    };

    Some(Health::Http {
        status,
        scheme,
        latency,
        server,
        title,
        is_html,
    })
}

/// Pull `<title>` out of a document without pretending to parse HTML.
pub fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?;
    let open = lower[start..].find('>')? + start + 1;
    let end = lower[open..].find("</title>")? + open;
    let raw = html.get(open..end)?;
    let clean = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() || clean.chars().count() > 120 {
        None
    } else {
        Some(clean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn target(port: u16) -> Target {
        Target::root(1, port, IpAddr::V4(Ipv4Addr::LOCALHOST), Kind::Web)
    }

    #[test]
    fn extracts_a_title_across_the_awkward_forms() {
        assert_eq!(
            extract_title("<html><head><title>Acme</title>").as_deref(),
            Some("Acme")
        );
        assert_eq!(
            extract_title("<TITLE class=\"x\">\n  spaced   out\n</TITLE>").as_deref(),
            Some("spaced out"),
            "case and whitespace are normalised"
        );
        assert_eq!(
            extract_title("<title></title>"),
            None,
            "empty is not a title"
        );
        assert_eq!(extract_title("<title>unclosed"), None);
        assert_eq!(extract_title("no title here"), None);
        assert_eq!(
            extract_title(&format!("<title>{}</title>", "x".repeat(500))),
            None,
            "an absurd title is rejected rather than shown"
        );
    }

    #[test]
    fn title_extraction_never_panics_on_multibyte_input() {
        for s in [
            "<title>日本語のページ</title>",
            "<title>é</title>",
            "<title>🎉",
        ] {
            let _ = extract_title(s);
        }
        assert_eq!(
            extract_title("<title>日本語</title>").as_deref(),
            Some("日本語")
        );
    }

    #[test]
    fn a_wildcard_bind_is_dialled_on_loopback() {
        let t = Target::root(1, 8080, IpAddr::from([0, 0, 0, 0]), Kind::Web);
        assert_eq!(t.socket().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn the_pool_runs_every_submitted_target() {
        let mut answers = HashMap::new();
        answers.insert(
            3000,
            Health::Open {
                latency: Duration::from_millis(1),
            },
        );
        let prober = ScriptedProber::new(answers);
        let calls = Arc::clone(&prober.calls);
        let pool = Pool::with_workers(Arc::new(prober), 4);

        for port in [3000, 3001, 3002] {
            assert!(pool.submit(target(port)));
        }
        let outcomes = pool.collect(3, Duration::from_secs(5));
        assert_eq!(outcomes.len(), 3);
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        assert_eq!(pool.queued(), 0, "queue drains to empty");
    }

    #[test]
    fn one_slow_probe_does_not_block_the_others() {
        let prober = ScriptedProber::new(HashMap::new()).with_delay(Duration::from_millis(200));
        let pool = Pool::with_workers(Arc::new(prober), 4);
        for port in 0..4u16 {
            pool.submit(target(3000 + port));
        }
        let started = Instant::now();
        let outcomes = pool.collect(4, Duration::from_secs(5));
        assert_eq!(outcomes.len(), 4);
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "four 200ms probes on four workers took {:?} — they serialised",
            started.elapsed()
        );
    }

    #[test]
    fn dropping_the_pool_joins_its_workers() {
        let pool = Pool::with_workers(Arc::new(ScriptedProber::new(HashMap::new())), 3);
        pool.submit(target(3000));
        let _ = pool.collect(1, Duration::from_secs(5));
        drop(pool); // Hangs here if a worker fails to notice the closed channel.
    }

    #[test]
    fn a_closed_port_reports_closed() {
        let prober =
            NetProber::with_timeouts(Duration::from_millis(200), Duration::from_millis(200));
        // Ephemeral ports can be reused between the bind and the probe, so try
        // a few and require one to come back closed rather than betting on one.
        let mut last = Health::Unknown;
        for _ in 0..5 {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("addr").port();
            drop(listener);
            last = prober.probe(&target(port)).health;
            if matches!(last, Health::Closed) {
                return;
            }
        }
        panic!("no unbound port reported Closed; last was {last:?}");
    }

    #[test]
    fn a_live_listener_reports_open() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let mut t = target(port);
        // A raw socket that never speaks HTTP: the TCP result is the answer.
        t.kind = Kind::Database;
        let health =
            NetProber::with_timeouts(Duration::from_millis(400), Duration::from_millis(400))
                .probe(&t)
                .health;
        assert!(matches!(health, Health::Open { .. }), "got {health:?}");
    }
}

#[cfg(test)]
mod banner_tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    fn target(port: u16, kind: Kind) -> Target {
        Target::root(1, port, IpAddr::V4(Ipv4Addr::LOCALHOST), kind)
    }

    /// Greets every connection, immediately, and concurrently — a prober makes
    /// more than one connection, and a server that handles them one at a time
    /// would be measuring its own queue rather than quarry's behaviour.
    fn greeting_server(greeting: &'static [u8]) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                std::thread::spawn(move || {
                    let _ = stream.write_all(greeting);
                    let _ = stream.flush();
                    std::thread::sleep(Duration::from_millis(400));
                });
            }
        });
        port
    }

    /// The case this was built for: OrbStack's sshd on 32222 said
    /// `SSH-2.0-OrbStack` and quarry reported `other · open`.
    #[test]
    fn a_service_that_greets_is_heard() {
        let port = greeting_server(b"SSH-2.0-OpenSSH_9.6\r\n");
        let mut t = target(port, Kind::System);
        t.handshake = None;
        let observed = NetProber::new().probe(&t);
        let banner = observed
            .banner
            .expect("the greeting should have been heard");
        assert!(
            String::from_utf8_lossy(&banner).starts_with("SSH-2.0-"),
            "got {banner:?}"
        );
    }

    /// The measurement that set the window: a banner arrives at once, so the
    /// window is a ceiling and not a cost.
    #[test]
    fn hearing_a_banner_costs_nothing() {
        let port = greeting_server(b"220 test ESMTP\r\n");
        let mut t = target(port, Kind::Mail);
        t.handshake = None;
        // Measured from the banner read itself, not from the whole probe: a
        // mail port also gets an HTTP attempt first, and that is not what this
        // is about.
        let stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let started = Instant::now();
        let banner = read_banner(&stream, BANNER_WINDOW);
        let elapsed = started.elapsed();
        assert!(banner.is_some(), "the greeting should have been heard");
        assert!(
            elapsed * 5 < BANNER_WINDOW,
            "a talkative server took {elapsed:?} of a {BANNER_WINDOW:?} window"
        );

        let observed = NetProber::new().probe(&t);
        assert!(
            observed.banner.is_some(),
            "and through the whole prober too"
        );
    }

    #[test]
    fn a_silent_service_gives_up_after_the_window_and_no_longer() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(8).filter_map(Result::ok).collect();
            std::thread::sleep(Duration::from_secs(4));
            drop(held);
        });
        let mut t = target(port, Kind::Database);
        t.handshake = None;
        let started = Instant::now();
        let observed = NetProber::new().probe(&t);
        assert!(observed.banner.is_none());
        assert!(matches!(observed.health, Health::Open { .. }));
        assert!(
            started.elapsed() < BANNER_WINDOW * 3,
            "waited {:?}",
            started.elapsed()
        );
    }

    /// An HTTP server is silent, and must not pay the banner window on the
    /// path that identifies it.
    #[test]
    fn an_http_service_never_waits_for_a_banner() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nhi",
                );
            }
        });
        let started = Instant::now();
        let observed = NetProber::new().probe(&target(port, Kind::Web));
        assert!(matches!(observed.health, Health::Http { status: 200, .. }));
        assert!(observed.banner.is_none());
        assert!(
            started.elapsed() < BANNER_WINDOW,
            "an HTTP service paid the banner window: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_handshake_runs_only_when_the_service_stays_silent() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 64];
                let n = stream.read(&mut buf).unwrap_or(0);
                if &buf[..n] == b"PING\r\n" {
                    let _ = stream.write_all(b"+PONG\r\n");
                }
            }
        });
        let mut t = target(port, Kind::Cache);
        t.handshake = Some("redis".into());
        let observed = NetProber::new().probe(&t);
        assert_eq!(
            observed.banner.as_deref(),
            Some(&b"+PONG\r\n"[..]),
            "the handshake reply should come back as evidence"
        );
    }
}
