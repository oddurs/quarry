//! Asking a service directly, for protocols that will not speak first.
//!
//! A port is a convention and a handshake is proof. Something else sitting on
//! 5432 currently reports as "PostgreSQL, healthy"; one round trip settles it.
//!
//! Every exchange here is **read-only and unauthenticated**. Nothing writes,
//! nothing logs in, nothing that could change the state of a database a
//! developer is in the middle of using. The reply is returned as bytes and fed
//! back through the signature table as evidence, exactly like a banner, so
//! adding a protocol is a table entry plus a function rather than a change to
//! the classifier.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Run a named handshake and return whatever came back.
pub fn run(name: &str, stream: &TcpStream, window: Duration) -> Option<Vec<u8>> {
    let request: Vec<u8> = match name {
        "redis" => b"PING\r\n".to_vec(),
        "memcached" => b"version\r\n".to_vec(),
        "postgres" => ssl_request(),
        "mongodb" => mongodb_hello(),
        "http" => b"GET / HTTP/1.0\r\n\r\n".to_vec(),
        // MySQL, AMQP, NATS and the SMTP family all greet on connect, so they
        // are handled by the banner read and never reach here.
        _ => return None,
    };
    exchange(stream, &request, window)
}

/// Whether quarry knows how to ask this protocol anything.
pub fn is_known(name: &str) -> bool {
    matches!(
        name,
        "redis" | "memcached" | "postgres" | "mongodb" | "http" | "mysql" | "amqp" | "mqtt"
    )
}

fn exchange(stream: &TcpStream, request: &[u8], window: Duration) -> Option<Vec<u8>> {
    let mut stream = stream.try_clone().ok()?;
    stream.set_write_timeout(Some(window)).ok()?;
    stream.set_read_timeout(Some(window)).ok()?;
    stream.write_all(request).ok()?;
    stream.flush().ok()?;

    let mut buf = [0u8; 512];
    match stream.read(&mut buf) {
        Ok(0) => None,
        Ok(n) => Some(buf[..n].to_vec()),
        Err(_) => None,
    }
}

/// PostgreSQL's `SSLRequest`: eight bytes, answered with a single `S` or `N`.
///
/// The cheapest possible question — it asks whether TLS is available, before
/// any authentication, and a server that answers with one of those two bytes
/// is a PostgreSQL server and nothing else.
fn ssl_request() -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&8u32.to_be_bytes());
    // 1234 << 16 | 5679, the magic SSL request code.
    out.extend_from_slice(&80877103u32.to_be_bytes());
    out
}

/// A minimal `hello` in the MongoDB wire protocol: an `OP_QUERY` against
/// `admin.$cmd` asking `{ isMaster: 1 }`, which every version answers without
/// authentication.
fn mongodb_hello() -> Vec<u8> {
    // BSON: { isMaster: 1 } as an int32 field.
    let mut doc = Vec::new();
    doc.push(0x10u8); // int32
    doc.extend_from_slice(b"isMaster\0");
    doc.extend_from_slice(&1i32.to_le_bytes());
    doc.push(0x00); // end of document
    let doc_len = (doc.len() + 4) as i32;
    let mut bson = doc_len.to_le_bytes().to_vec();
    bson.extend_from_slice(&doc);

    let collection = b"admin.$cmd\0";
    let body_len = 4 + collection.len() + 4 + 4 + bson.len();
    let total = 16 + body_len;

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as i32).to_le_bytes()); // messageLength
    out.extend_from_slice(&1i32.to_le_bytes()); // requestID
    out.extend_from_slice(&0i32.to_le_bytes()); // responseTo
    out.extend_from_slice(&2004i32.to_le_bytes()); // OP_QUERY
    out.extend_from_slice(&0i32.to_le_bytes()); // flags
    out.extend_from_slice(collection);
    out.extend_from_slice(&0i32.to_le_bytes()); // numberToSkip
    out.extend_from_slice(&1i32.to_le_bytes()); // numberToReturn
    out.extend_from_slice(&bson);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A server that records what it was asked and replies with `reply`.
    fn scripted(reply: &'static [u8]) -> (u16, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 512];
                let n = stream.read(&mut buf).unwrap_or(0);
                let _ = tx.send(buf[..n].to_vec());
                let _ = stream.write_all(reply);
            }
        });
        (port, rx)
    }

    fn ask(port: u16, name: &str) -> Option<Vec<u8>> {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        run(name, &stream, Duration::from_secs(2))
    }

    #[test]
    fn redis_is_asked_to_ping_and_its_answer_comes_back() {
        let (port, asked) = scripted(b"+PONG\r\n");
        let reply = ask(port, "redis").expect("a reply");
        assert_eq!(
            asked
                .recv_timeout(Duration::from_secs(2))
                .expect("recorded"),
            b"PING\r\n"
        );
        assert_eq!(reply, b"+PONG\r\n");
    }

    #[test]
    fn memcached_is_asked_for_its_version() {
        let (port, asked) = scripted(b"VERSION 1.6.21\r\n");
        let reply = ask(port, "memcached").expect("a reply");
        assert_eq!(
            asked
                .recv_timeout(Duration::from_secs(2))
                .expect("recorded"),
            b"version\r\n"
        );
        assert!(String::from_utf8_lossy(&reply).starts_with("VERSION"));
    }

    /// The whole point: a server that is not PostgreSQL does not answer like
    /// one, and quarry stops claiming it is.
    #[test]
    fn postgres_is_asked_whether_it_speaks_tls() {
        let (port, asked) = scripted(b"N");
        let reply = ask(port, "postgres").expect("a reply");
        let request = asked
            .recv_timeout(Duration::from_secs(2))
            .expect("recorded");
        assert_eq!(request.len(), 8, "SSLRequest is eight bytes");
        assert_eq!(&request[0..4], &8u32.to_be_bytes(), "length prefix");
        assert_eq!(&request[4..8], &80877103u32.to_be_bytes(), "the magic code");
        assert_eq!(reply, b"N");
    }

    #[test]
    fn the_postgres_request_never_carries_credentials() {
        let request = ssl_request();
        assert_eq!(request.len(), 8);
        // Eight bytes cannot contain a username, and this asserts it stays that
        // way: this handshake must never grow into a login attempt.
        assert!(!request.iter().any(|b| b.is_ascii_alphabetic()));
    }

    #[test]
    fn the_mongodb_query_is_well_formed() {
        let msg = mongodb_hello();
        let declared = i32::from_le_bytes(msg[0..4].try_into().expect("4 bytes"));
        assert_eq!(
            declared as usize,
            msg.len(),
            "the length prefix must match the message"
        );
        let opcode = i32::from_le_bytes(msg[12..16].try_into().expect("4 bytes"));
        assert_eq!(opcode, 2004, "OP_QUERY");
        assert!(
            msg.windows(10).any(|w| w == b"admin.$cmd"),
            "asks the admin database"
        );
        assert!(msg.windows(8).any(|w| w == b"isMaster"));
    }

    #[test]
    fn an_unknown_handshake_asks_nothing() {
        let (port, asked) = scripted(b"anything");
        assert_eq!(ask(port, "not-a-protocol"), None);
        // The server sees the connection close without a byte written, which
        // arrives as an empty read rather than as no read at all.
        let seen = asked
            .recv_timeout(Duration::from_millis(500))
            .unwrap_or_default();
        assert!(
            seen.is_empty(),
            "an unknown handshake must not send anything at all, sent {seen:?}"
        );
    }

    #[test]
    fn a_silent_server_yields_nothing_rather_than_hanging() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(4).filter_map(Result::ok).collect();
            std::thread::sleep(Duration::from_secs(3));
            drop(held);
        });
        let started = std::time::Instant::now();
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let reply = run("redis", &stream, Duration::from_millis(200));
        assert_eq!(reply, None);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "waited {:?} on a silent server",
            started.elapsed()
        );
    }

    #[test]
    fn every_probe_named_in_the_shipped_table_is_one_we_know() {
        for sig in crate::signature::Registry::builtin().iter() {
            if let Some(probe) = &sig.probe {
                assert!(
                    is_known(probe),
                    "{} names handshake {probe:?}, which does not exist",
                    sig.name
                );
            }
        }
    }
}
