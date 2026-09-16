//! Attributing a published port to the container behind it.
//!
//! A container's port belongs, as far as the operating system is concerned, to
//! whatever host process published it. On the machine this was written on that
//! meant two ports labelled `OrbStack` while a container was running — the
//! runtime's name instead of the container's, the image's, or the project's.
//! Anyone running their stack in Compose sees one row per published port, all
//! of them named after the runtime.
//!
//! The daemon knows better, and will say so over a unix socket with no client
//! library and no dependency. Compose writes the labels that make the rest
//! work: `com.docker.compose.project.working_dir` is a directory on disk, which
//! feeds straight into the same repository resolver a process's working
//! directory does — so a container and a process started from one repository
//! land in the same group.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// Long enough for a daemon that is busy, short enough that a wedged one costs
/// a scan rather than the session.
const TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    /// The daemon's own id. Names collide across runtimes; this does not, and
    /// it is what the API wants in a path.
    pub id: String,
    /// The daemon that owns it. Two runtimes can be live at once on different
    /// sockets, so an instruction has to go back to the one that answered.
    pub socket: PathBuf,
    pub name: String,
    pub image: String,
    /// `running`, `restarting`, `exited`, …
    pub state: String,
    /// The daemon's own health verdict, where the image declares a healthcheck.
    pub health: Option<String>,
    /// The Compose project this belongs to, if any.
    pub project: Option<String>,
    /// The Compose service name, which is what people call it.
    pub service: Option<String>,
    /// Where the Compose file lives — a real directory, resolvable to a repo.
    pub working_dir: Option<PathBuf>,
}

impl Container {
    /// What to call it: the Compose service name where there is one, since that
    /// is the name in the file the user wrote.
    pub fn display_name(&self) -> &str {
        self.service.as_deref().unwrap_or(&self.name)
    }

    /// A name somebody meant, or nothing.
    ///
    /// The Compose service name first: it is the word written in the file, and
    /// it is chosen to say what the thing is — `postgres`, `grpc`, `worker`.
    /// Then the container's own name, unless Docker invented it.
    pub fn chosen_name(&self) -> Option<&str> {
        if let Some(service) = self.service.as_deref() {
            return Some(service);
        }
        (!looks_generated(&self.name)).then_some(self.name.as_str())
    }

    /// The image's bare repository name: `mongo:7` is `mongo`, and
    /// `quay.io/minio/minio:latest` is `minio`.
    ///
    /// The registry and the tag are stripped because both are matched against
    /// the signature table, and both can carry a name that means something
    /// there by accident — an image pulled from `redis.example.com/anything`
    /// is not a Redis.
    pub fn image_name(&self) -> &str {
        let without_tag = match self.image.rsplit_once(':') {
            // A registry with a port, not a tag: `host:5000/image`.
            Some((head, tail)) if !tail.contains('/') => head,
            _ => self.image.as_str(),
        };
        without_tag.rsplit('/').next().unwrap_or(without_tag)
    }

    pub fn is_healthy(&self) -> bool {
        match self.health.as_deref() {
            Some(h) => h == "healthy",
            None => self.state == "running",
        }
    }
}

/// Whether Docker made this name up.
///
/// A container started without `--name` is given an adjective and a scientist
/// — `beautiful_heisenberg`, `goofy_diffie` — from two word lists. Seven of
/// them on one machine, all running the same image, and not one of the names
/// says what any of them is doing.
///
/// Detected by shape rather than by shipping both word lists: two lowercase
/// words joined by an underscore. Compose uses hyphens and so does almost
/// everyone naming a container by hand, so the shape is close to exclusive. A
/// container deliberately named `my_app` is read as generated and shown by its
/// image instead — which is a mild wrong answer, and arguably a more useful one.
fn looks_generated(name: &str) -> bool {
    let Some((adjective, surname)) = name.split_once('_') else {
        return false;
    };
    let word = |w: &str| w.len() >= 3 && w.chars().all(|c| c.is_ascii_lowercase());
    word(adjective) && word(surname)
}

/// Published host port → the container behind it.
#[derive(Debug, Default)]
pub struct Containers {
    by_port: HashMap<u16, Container>,
}

impl Containers {
    pub fn get(&self, port: u16) -> Option<&Container> {
        self.by_port.get(&port)
    }

    pub fn is_empty(&self) -> bool {
        self.by_port.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_port.len()
    }

    /// Ask every daemon that answers, and merge what they say.
    ///
    /// Not the first one that answers: this machine runs OrbStack and Docker
    /// Desktop at the same time, on different sockets, and a container started
    /// against one is invisible to the other. Stopping at the first reply meant
    /// reporting no containers while one was plainly running.
    ///
    /// No daemon at all is not a failure — most machines are not running one —
    /// so that reports nothing rather than an error nobody can act on.
    pub fn query() -> Containers {
        let mut by_port: HashMap<u16, Container> = HashMap::new();
        let mut seen: Vec<PathBuf> = Vec::new();

        for socket in socket_paths() {
            if !socket.exists() {
                continue;
            }
            // A symlink and its target are one daemon asked twice.
            let real = std::fs::canonicalize(&socket).unwrap_or_else(|_| socket.clone());
            if seen.contains(&real) {
                continue;
            }
            seen.push(real);

            match fetch(&socket) {
                // A socket that connects but answers with nothing is not an
                // answer, and the search should carry on to one that would.
                Ok(body) if body.trim_start().starts_with('[') => {
                    let found = parse(&body, &socket);
                    if !found.by_port.is_empty() {
                        crate::diag::info(
                            "docker",
                            format!("{} published port(s) via {}", found.len(), socket.display()),
                        );
                    }
                    // First daemon to claim a port keeps it; two runtimes
                    // cannot both be publishing the same one.
                    for (port, container) in found.by_port {
                        by_port.entry(port).or_insert(container);
                    }
                }
                Ok(_) => crate::diag::warn(
                    "docker",
                    format!("{}: no container list in the reply", socket.display()),
                ),
                Err(e) => crate::diag::warn("docker", format!("{}: {e}", socket.display())),
            }
        }
        Containers { by_port }
    }
}

/// Every place a Docker-compatible daemon puts its socket. OrbStack, Colima and
/// Rancher Desktop all answer the same API; the first one that does wins.
pub fn socket_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(host) = std::env::var_os("DOCKER_HOST") {
        let host = host.to_string_lossy().to_string();
        if let Some(path) = host.strip_prefix("unix://") {
            paths.push(PathBuf::from(path));
        }
    }
    paths.push(PathBuf::from("/var/run/docker.sock"));
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        paths.push(home.join(".docker/run/docker.sock"));
        paths.push(home.join(".orbstack/run/docker.sock"));
        paths.push(home.join(".colima/default/docker.sock"));
        paths.push(home.join(".rd/docker.sock"));
        // Podman's machine socket on macOS.
        paths.push(home.join(".local/share/containers/podman/machine/podman.sock"));
    }
    // Rootless Podman on Linux, where the runtime directory is per-user.
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(runtime) => paths.push(PathBuf::from(runtime).join("podman/podman.sock")),
        None => paths.push(PathBuf::from("/run/podman/podman.sock")),
    }
    paths
}

/// An HTTP GET over a unix socket, by hand, because the API is plain HTTP and
/// this is one request.
///
/// The body is read by `Content-Length` rather than to end-of-stream. Reading
/// to the end waits for the daemon to close, and the daemon does not: it holds
/// the connection open regardless of what the request asked for, so a naive
/// `read_to_end` cost the full timeout — one and a half seconds — on every
/// scan.
fn fetch(socket: &std::path::Path) -> Result<String, String> {
    let reply = request(socket, "GET", "/containers/json", TIMEOUT)?;
    Ok(reply
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .ok_or("no body in the daemon\u{2019}s reply")?)
}

/// One request, headers and all, so a caller that needs the status line can
/// have it. Returns the whole reply.
fn request(
    socket: &std::path::Path,
    method: &str,
    path: &str,
    timeout: Duration,
) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    stream
        // HTTP/1.0 deliberately: asked over 1.1 the daemon answers with chunked
        // framing, and this is not the place for a chunked decoder.
        .write_all(
            format!("{method} {path} HTTP/1.0\r\nHost: docker\r\nAccept: application/json\r\n\r\n")
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;

    let mut raw = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut header_end = None;

    loop {
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..n]);

        if header_end.is_none() {
            header_end = find(&raw, b"\r\n\r\n").map(|i| i + 4);
        }
        let Some(start) = header_end else {
            if raw.len() > 64 * 1024 {
                return Err("headers never ended".into());
            }
            continue;
        };
        let head = String::from_utf8_lossy(&raw[..start]).to_lowercase();
        // 204 and 304 cannot carry a body, so the headers are the whole reply.
        // Without this the read would sit here until the timeout: the daemon
        // keeps the connection open whatever the request asked for, which is
        // also why the body below is framed by Content-Length rather than by
        // waiting for a close.
        if matches!(status_code(&head), Some(204 | 304)) {
            return Ok(String::from_utf8_lossy(&raw[..start]).to_string());
        }
        if let Some(len) = content_length(&head)
            && raw.len() >= start + len
        {
            return Ok(String::from_utf8_lossy(&raw[..start + len]).to_string());
        }
        if raw.len() > 8 * 1024 * 1024 {
            return Err("reply too large".into());
        }
    }

    if header_end.is_none() {
        return Err("the daemon closed without answering".into());
    }
    Ok(String::from_utf8_lossy(&raw).to_string())
}

/// A stop or a restart waits for the container's own grace period before the
/// daemon gives up and kills it, so this cannot share the scan's timeout.
const ACT_TIMEOUT: Duration = Duration::from_secs(30);

/// Tell the daemon to do something to a container.
///
/// Addressed to the socket the container was found on rather than to whatever
/// `docker` on PATH happens to point at. On a machine running two runtimes
/// those are different daemons, and the CLI would be talking to the wrong one.
pub fn act(container: &Container, verb: &str) -> Result<(), String> {
    if container.id.is_empty() {
        return Err("the daemon gave this container no id".into());
    }
    // `t` is the grace period before the daemon escalates to SIGKILL. Ten
    // seconds is the daemon's own default; naming it keeps our deadline and
    // the daemon's from drifting apart.
    let path = format!("/containers/{}/{verb}?t=10", container.id);
    let reply = request(&container.socket, "POST", &path, ACT_TIMEOUT)?;
    match status_code(&reply) {
        // 204 is done. 304 is "already in that state", which is the outcome
        // the user asked for even though nothing happened.
        Some(204 | 304) => Ok(()),
        Some(404) => Err("the daemon no longer has this container".into()),
        Some(code) => Err(format!("daemon answered {code}: {}", body_of(&reply))),
        None => Err("the daemon answered with no status line".into()),
    }
}

fn status_code(reply: &str) -> Option<u16> {
    reply.split_whitespace().nth(1)?.parse().ok()
}

/// The message body, for an error worth repeating. The daemon puts its
/// explanation in a JSON `message` field.
fn body_of(reply: &str) -> String {
    let body = reply.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    serde_json::from_str::<serde_json::Value>(body.trim())
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn content_length(headers_lowercase: &str) -> Option<usize> {
    headers_lowercase
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
}

/// Parse `/containers/json`. Tolerant by design: an unfamiliar shape costs the
/// container it appeared in, not the whole answer.
pub fn parse(body: &str, socket: &std::path::Path) -> Containers {
    let mut by_port = HashMap::new();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim()) else {
        return Containers { by_port };
    };
    let Some(list) = value.as_array() else {
        return Containers { by_port };
    };

    for item in list {
        let labels = item.get("Labels").and_then(|l| l.as_object());
        let label = |key: &str| {
            labels
                .and_then(|l| l.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };

        let name = item
            .get("Names")
            .and_then(|n| n.as_array())
            .and_then(|n| n.first())
            .and_then(|n| n.as_str())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_default();

        // `Status` reads "Up 2 hours (healthy)"; the parenthetical is the
        // daemon's own verdict and worth more than anything quarry can infer.
        let status = item.get("Status").and_then(|s| s.as_str()).unwrap_or("");
        let health = ["healthy", "unhealthy", "starting"]
            .into_iter()
            .find(|h| status.contains(h))
            .map(str::to_string);

        let container = Container {
            id: item
                .get("Id")
                .and_then(|i| i.as_str())
                .unwrap_or_default()
                .to_string(),
            socket: socket.to_path_buf(),
            name,
            image: item
                .get("Image")
                .and_then(|i| i.as_str())
                .unwrap_or_default()
                .to_string(),
            state: item
                .get("State")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string(),
            health,
            project: label("com.docker.compose.project"),
            service: label("com.docker.compose.service"),
            working_dir: label("com.docker.compose.project.working_dir").map(PathBuf::from),
        };

        for port in item
            .get("Ports")
            .and_then(|p| p.as_array())
            .unwrap_or(&vec![])
        {
            // Only published ports matter: an unpublished one is not reachable
            // from here and quarry never saw it.
            let Some(public) = port.get("PublicPort").and_then(|p| p.as_u64()) else {
                continue;
            };
            if let Ok(public) = u16::try_from(public) {
                by_port.insert(public, container.clone());
            }
        }
    }
    Containers { by_port }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {
        "Id": "abc123",
        "Names": ["/acme-db-1"],
        "Image": "postgres:16",
        "State": "running",
        "Status": "Up 2 hours (healthy)",
        "Ports": [
          {"IP":"0.0.0.0","PrivatePort":5432,"PublicPort":5433,"Type":"tcp"},
          {"PrivatePort":9999,"Type":"tcp"}
        ],
        "Labels": {
          "com.docker.compose.project": "acme",
          "com.docker.compose.service": "db",
          "com.docker.compose.project.working_dir": "/Users/someone/Code/acme"
        }
      },
      {
        "Id": "def456",
        "Names": ["/lonely"],
        "Image": "redis:7",
        "State": "restarting",
        "Status": "Restarting (1) 5 seconds ago",
        "Ports": [{"PrivatePort":6379,"PublicPort":6379,"Type":"tcp"}],
        "Labels": {}
      }
    ]"#;

    #[test]
    fn a_reply_is_read_by_its_status_line() {
        assert_eq!(status_code("HTTP/1.0 204 No Content\r\n\r\n"), Some(204));
        assert_eq!(status_code("HTTP/1.0 304 Not Modified\r\n\r\n"), Some(304));
        assert_eq!(status_code("nonsense"), None);
    }

    /// The daemon's own words are worth repeating — "port is already
    /// allocated" tells a user what to do; "409" does not.
    #[test]
    fn a_refusal_carries_the_daemon_s_explanation() {
        let reply = "HTTP/1.0 409 Conflict\r\nContent-Type: application/json\r\n\r\n\
                     {\"message\":\"port is already allocated\"}";
        assert_eq!(body_of(reply), "port is already allocated");
        assert_eq!(body_of("HTTP/1.0 500 x\r\n\r\nplain text"), "plain text");
    }

    #[test]
    fn a_container_with_no_id_is_not_acted_on() {
        let c = Container {
            id: String::new(),
            socket: "/var/run/docker.sock".into(),
            name: "orphan".into(),
            image: "nginx".into(),
            state: "running".into(),
            health: None,
            project: None,
            service: None,
            working_dir: None,
        };
        // No socket is touched: the guard comes first, so this cannot hang on
        // a machine with a daemon running.
        let err = act(&c, "restart").unwrap_err();
        assert!(err.contains("no id"), "{err}");
    }

    #[test]
    fn a_container_carries_the_daemon_that_answered_for_it() {
        let socket = std::path::Path::new("/tmp/quarry-test/docker.sock");
        let c = parse(SAMPLE, socket);
        let c = c.get(5433).expect("a published port");
        assert_eq!(
            c.socket, socket,
            "an instruction has to go back to the same daemon"
        );
        assert!(!c.id.is_empty(), "no id to address it by");
    }

    #[test]
    fn a_published_port_finds_its_container() {
        let c = parse(SAMPLE, std::path::Path::new("/var/run/docker.sock"));
        let db = c.get(5433).expect("5433 is published");
        assert_eq!(db.name, "acme-db-1");
        assert_eq!(db.image, "postgres:16");
        assert_eq!(
            db.display_name(),
            "db",
            "the Compose service name is the one people use"
        );
        assert!(db.is_healthy());
    }

    /// The point of the whole exercise: a container resolves to a directory,
    /// and a directory resolves to a repository.
    #[test]
    fn a_compose_container_carries_the_directory_it_came_from() {
        let c = parse(SAMPLE, std::path::Path::new("/var/run/docker.sock"));
        let db = c.get(5433).expect("present");
        assert_eq!(db.project.as_deref(), Some("acme"));
        assert_eq!(
            db.working_dir.as_deref(),
            Some(std::path::Path::new("/Users/someone/Code/acme"))
        );
    }

    #[test]
    fn an_unpublished_port_is_not_reachable_and_not_listed() {
        let c = parse(SAMPLE, std::path::Path::new("/var/run/docker.sock"));
        assert!(c.get(9999).is_none(), "9999 was never published");
    }

    #[test]
    fn the_daemons_own_health_verdict_is_used() {
        let c = parse(SAMPLE, std::path::Path::new("/var/run/docker.sock"));
        assert!(
            !c.get(6379).expect("present").is_healthy(),
            "restarting is not healthy"
        );
        assert_eq!(c.get(6379).expect("present").display_name(), "lonely");
    }

    #[test]
    fn content_length_is_found_however_it_is_cased() {
        let head =
            "http/1.1 200 ok\r\ncontent-type: application/json\r\ncontent-length: 1234\r\n\r\n";
        assert_eq!(content_length(head), Some(1234));
        assert_eq!(content_length("http/1.1 200 ok\r\n\r\n"), None);
    }

    /// The daemon holds the connection open regardless of what we ask, so
    /// reading to end-of-stream cost the full timeout on every single scan.
    #[test]
    fn asking_the_daemon_is_fast_or_absent() {
        let started = std::time::Instant::now();
        let _ = Containers::query();
        assert!(
            started.elapsed() < Duration::from_millis(400),
            "querying the daemon took {:?}, which is charged to every scan",
            started.elapsed()
        );
    }

    #[test]
    fn nonsense_costs_nothing() {
        for body in [
            "",
            "not json",
            "{}",
            "[]",
            "[{}]",
            "[{\"Ports\":\"wrong\"}]",
        ] {
            let c = parse(body, std::path::Path::new("/var/run/docker.sock"));
            assert!(c.is_empty(), "{body:?} produced containers");
        }
    }

    #[test]
    fn one_odd_container_does_not_cost_the_others() {
        let body = r#"[{"Ports":"wrong"},{"Names":["/ok"],"Ports":[{"PublicPort":8080}]}]"#;
        let c = parse(body, std::path::Path::new("/var/run/docker.sock"));
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(8080).expect("present").name, "ok");
    }

    #[test]
    fn every_runtime_socket_is_looked_for() {
        let paths = socket_paths();
        let joined = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        for expected in ["/var/run/docker.sock", "orbstack", "colima", "podman"] {
            assert!(
                joined.contains(expected),
                "no path for {expected}: {joined}"
            );
        }
    }

    /// Against whatever is actually running here, if anything is.
    #[test]
    fn it_talks_to_a_real_daemon_when_there_is_one() {
        let found = Containers::query();
        for port in 1..=u16::MAX {
            if let Some(c) = found.get(port) {
                assert!(!c.name.is_empty(), "a container with no name");
                assert!(!c.image.is_empty(), "a container with no image");
            }
        }
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    fn named(image: &str) -> Container {
        Container {
            id: "x".into(),
            socket: "/var/run/docker.sock".into(),
            name: "c".into(),
            image: image.to_string(),
            state: "running".into(),
            health: None,
            project: None,
            service: None,
            working_dir: None,
        }
    }

    /// The image is matched against the signature table, so a registry
    /// hostname carrying a name that means something there would classify an
    /// image by where it was pulled from.
    #[test]
    fn an_image_is_reduced_to_its_repository_name() {
        assert_eq!(named("mongo:7").image_name(), "mongo");
        assert_eq!(named("postgres:16-alpine").image_name(), "postgres");
        assert_eq!(named("quay.io/minio/minio:latest").image_name(), "minio");
        assert_eq!(named("nginx").image_name(), "nginx");
        assert_eq!(named("ghcr.io/acme/web:1.2.3").image_name(), "web");
    }

    /// A registry with a port looks like a tag and is not one.
    #[test]
    fn a_registry_port_is_not_a_tag() {
        assert_eq!(named("registry:5000/team/api").image_name(), "api");
    }

    /// The whole point: this must not be read as a Redis.
    #[test]
    fn a_registry_named_after_a_service_does_not_name_the_image() {
        assert_eq!(
            named("redis.example.com/acme/billing:2").image_name(),
            "billing"
        );
    }
}

#[cfg(test)]
mod naming_tests {
    use super::*;

    fn container(name: &str, service: Option<&str>, image: &str) -> Container {
        Container {
            id: "x".into(),
            socket: "/var/run/docker.sock".into(),
            name: name.to_string(),
            image: image.to_string(),
            state: "running".into(),
            health: None,
            project: None,
            service: service.map(str::to_string),
            working_dir: None,
        }
    }

    /// The word written in the Compose file, which is chosen to say what the
    /// thing is.
    #[test]
    fn a_compose_service_name_is_a_name_somebody_meant() {
        let c = container("stack-postgres-1", Some("postgres"), "postgres:16");
        assert_eq!(c.chosen_name(), Some("postgres"));
    }

    /// `--name my-api` is somebody saying what it is.
    #[test]
    fn a_name_given_by_hand_is_kept() {
        assert_eq!(
            container("my-api", None, "nginx:alpine").chosen_name(),
            Some("my-api")
        );
        assert_eq!(
            container("redis1", None, "redis:7").chosen_name(),
            Some("redis1")
        );
        // Compose's own naming uses hyphens and is not generated.
        assert_eq!(
            container("quarry-taxonomy-redis-1", None, "redis:7").chosen_name(),
            Some("quarry-taxonomy-redis-1")
        );
    }

    /// Seven of these on one machine, all running the same image, and not one
    /// of the names says what any of them is doing.
    #[test]
    fn a_name_docker_invented_is_no_name_at_all() {
        for invented in [
            "beautiful_heisenberg",
            "goofy_diffie",
            "dazzling_goldstine",
            "intelligent_mcnulty",
        ] {
            assert_eq!(
                container(invented, None, "rust:1-slim").chosen_name(),
                None,
                "{invented} was taken for a name somebody chose"
            );
        }
    }

    /// The shape is close to exclusive but not quite. Stated rather than
    /// pretended away.
    #[test]
    fn a_hand_written_name_of_the_same_shape_is_read_as_generated() {
        // Two real lowercase words joined by an underscore is the shape, and
        // this one is a person's. It will be shown by its image instead.
        assert_eq!(container("web_server", None, "nginx").chosen_name(), None);

        // Anything else about the name is enough to keep it: a short word, a
        // digit, a capital, a hyphen, or more than one underscore.
        for safe in ["my_app", "app_2", "My_App", "my-app", "a_b_c"] {
            assert!(
                container(safe, None, "nginx").chosen_name().is_some(),
                "{safe} was taken for a name Docker invented"
            );
        }
    }
}
