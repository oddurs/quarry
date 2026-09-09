//! Probing against real sockets. Hermetic: every listener is bound on loopback
//! with an ephemeral port inside the test itself, so nothing depends on what
//! the developer happens to be running.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::time::Duration;

use quarry::model::{Health, Kind};
use quarry::probe::{NetProber, Prober, Target};

/// Serve the same canned response to every connection, for the life of the
/// test binary. A probe may open more than one connection, so a server that
/// accepts exactly once would be testing the wrong thing.
fn serve(body: &'static str, headers: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            });
        }
    });
    port
}

fn target(port: u16, kind: Kind) -> Target {
    Target::root(
        std::process::id(),
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        kind,
    )
}

fn prober() -> NetProber {
    NetProber::with_timeouts(Duration::from_millis(500), Duration::from_secs(2))
}

#[test]
fn reads_status_title_and_server_from_a_real_response() {
    let port = serve(
        "<html><head><title>Acme — Dashboard</title></head><body>hi</body></html>",
        "content-type: text/html; charset=utf-8\r\nserver: test-server/1.0\r\n",
    );
    let health = prober().probe(&target(port, Kind::Web)).health;
    match health {
        Health::Http {
            status,
            title,
            server,
            is_html,
            scheme,
            ..
        } => {
            assert_eq!(status, 200);
            assert_eq!(scheme, "http");
            assert!(is_html);
            assert_eq!(title.as_deref(), Some("Acme — Dashboard"));
            assert_eq!(server.as_deref(), Some("test-server/1.0"));
        }
        other => panic!("expected an HTTP result, got {other:?}"),
    }
}

#[test]
fn a_json_endpoint_reports_no_title() {
    let port = serve(r#"{"ok":true}"#, "content-type: application/json\r\n");
    match prober().probe(&target(port, Kind::Api)).health {
        Health::Http { title, is_html, .. } => {
            assert!(!is_html);
            assert_eq!(title, None, "we do not invent a title for JSON");
        }
        other => panic!("expected an HTTP result, got {other:?}"),
    }
}

#[test]
fn a_socket_that_never_answers_still_reports_open() {
    // Accepts the connection and then says nothing at all.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        let held: Vec<_> = listener.incoming().take(4).filter_map(Result::ok).collect();
        std::thread::sleep(Duration::from_secs(4));
        drop(held);
    });

    let started = std::time::Instant::now();
    let health = prober().probe(&target(port, Kind::Web)).health;
    assert!(
        matches!(health, Health::Open { .. }),
        "a silent socket is open, not broken: {health:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "the probe hung for {:?} instead of timing out",
        started.elapsed()
    );
}

#[test]
fn a_non_http_service_is_not_asked_to_speak_http() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let health = prober().probe(&target(port, Kind::Database)).health;
    assert!(matches!(health, Health::Open { .. }), "got {health:?}");
    drop(listener);
}

#[test]
fn garbage_on_the_wire_does_not_crash_the_prober() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = stream.write_all(&[0xff, 0x00, 0xfe, 0x01, 0x7f, 0x80]);
        }
    });
    let health = prober().probe(&target(port, Kind::Web)).health;
    assert!(
        matches!(health, Health::Open { .. } | Health::Http { .. }),
        "got {health:?}"
    );
}

/// A dev server that 404s on `/` while being perfectly healthy is the reason
/// the health path is configurable.
#[test]
fn a_configured_health_path_is_the_one_requested() {
    use std::sync::mpsc;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            let _ = tx.send(path.clone());
            // Healthy only on /healthz, the way a real dev server behaves.
            let (code, reason) = if path == "/healthz" {
                (200, "OK")
            } else {
                (404, "Not Found")
            };
            let body = "{}";
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 {code} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });

    let mut t = target(port, Kind::Web);
    t.path = "/healthz".into();
    let health = prober().probe(&t).health;

    let requested = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("the server saw a request");
    assert_eq!(requested, "/healthz", "quarry asked for the wrong path");
    match health {
        Health::Http { status, .. } => assert_eq!(status, 200, "should read as healthy"),
        other => panic!("expected an HTTP result, got {other:?}"),
    }
}
