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
        "mqtt" => mqtt_connect(),
        "amqp" => AMQP_HEADER.to_vec(),
        "kafka" => kafka_api_versions(),
        "dns" => dns_query(),
        // MySQL, NATS and the SMTP family greet on connect, so they are
        // handled by the banner read and never reach here.
        _ => return None,
    };
    exchange(stream, &request, window)
}

/// Whether quarry knows how to ask this protocol anything.
pub fn is_known(name: &str) -> bool {
    matches!(
        name,
        "redis"
            | "memcached"
            | "postgres"
            | "mongodb"
            | "http"
            | "mysql"
            | "amqp"
            | "mqtt"
            | "kafka"
            | "dns"
    )
}

/// Whether a reply is the one this protocol gives.
///
/// This is the whole point of the module. A port is a convention: something
/// else on 5432 is reported as "PostgreSQL, healthy" on the strength of the
/// number alone. A handshake is proof, and proof means knowing what the right
/// answer looks like — not merely that some bytes came back.
///
/// Deliberately generous about *which* right answer. A Redis that replies
/// `-NOAUTH` is a Redis, and a broker that rejects our AMQP version by naming
/// its own has still identified itself.
pub fn confirms(name: &str, reply: &[u8]) -> bool {
    match name {
        // `+PONG`, or any error: an error in RESP is still RESP.
        "redis" => starts_with(reply, b"+PONG") || starts_with(reply, b"-"),
        "memcached" => starts_with(reply, b"VERSION") || starts_with(reply, b"ERROR"),
        // Exactly one byte, `S` or `N` — the length matters as much as the
        // letter. Testing the first byte alone accepted `SSH-2.0-OpenSSH_9.6`
        // as proof of PostgreSQL, which is the precise failure this function
        // exists to prevent. An older server may answer an ErrorResponse
        // instead: `E`, then a big-endian length that has to be plausible.
        "postgres" => match reply {
            [b'S'] | [b'N'] => true,
            [b'E', a, b, c, d, ..] => {
                let len = i32::from_be_bytes([*a, *b, *c, *d]);
                (4..30_000).contains(&len)
            }
            _ => false,
        },
        // An OP_REPLY: opcode 1, in the twelfth to sixteenth bytes of the
        // header, little-endian like the rest of the wire protocol.
        "mongodb" => opcode(reply) == Some(1),
        "http" => starts_with(reply, b"HTTP/"),
        // CONNACK, the only packet type a broker sends in answer to CONNECT.
        "mqtt" => reply.first() == Some(&0x20),
        // Either a method frame, or the header echoed back to say "not that
        // version, this one" — which identifies the broker just as well.
        "amqp" => reply.first() == Some(&0x01) || starts_with(reply, b"AMQP"),
        // The correlation id we sent, echoed after the length prefix.
        "kafka" => reply.get(4..8) == Some(&CORRELATION_ID.to_be_bytes()[..]),
        // Our transaction id, after the two-byte length prefix TCP DNS uses,
        // and the response bit set.
        "dns" => {
            reply.get(2..4) == Some(&DNS_ID.to_be_bytes()[..])
                && reply.get(4).is_some_and(|f| f & 0x80 != 0)
        }
        // A protocol we have no proof for cannot disprove anything either.
        _ => true,
    }
}

/// A version string, where the protocol volunteered one.
///
/// Only from bytes already in hand. Redis would give one for a second `INFO
/// server` round trip and that is not worth a second round trip; memcached and
/// MySQL put it in the reply we already asked for.
pub fn version(name: &str, reply: &[u8]) -> Option<String> {
    let text = match name {
        "memcached" => printable(reply).strip_prefix("VERSION ")?.to_string(),
        // The greeting is a packet header, a protocol byte, then the version as
        // a NUL-terminated string.
        "mysql" => {
            let rest = reply.get(5..)?;
            let end = rest.iter().position(|b| *b == 0)?;
            printable(&rest[..end])
        }
        // A banner that names itself — `SSH-2.0-OpenSSH_9.6`, `220 ProFTPD
        // 1.3.8` — carries a version in plain sight. Anything else does not,
        // and guessing from arbitrary bytes would invent one.
        _ => return looks_like_a_version(&printable(reply)),
    };
    let text = text.trim().to_string();
    (!text.is_empty() && text.len() <= 64).then_some(text)
}

/// The leading run of printable ASCII, which is as much of a binary reply as
/// can be read as text without inventing anything.
fn printable(reply: &[u8]) -> String {
    reply
        .iter()
        .take_while(|b| (0x20..0x7f).contains(*b) || **b == b'\r' || **b == b'\n')
        .map(|b| *b as char)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The first `1.2` or `1.2.3` in a line, with whatever word carries it.
fn looks_like_a_version(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .find(|word| {
            let mut parts = word.split('.');
            let enough = parts.clone().count() >= 2;
            enough && parts.all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
}

fn starts_with(reply: &[u8], prefix: &[u8]) -> bool {
    reply.len() >= prefix.len() && &reply[..prefix.len()] == prefix
}

fn opcode(reply: &[u8]) -> Option<i32> {
    let bytes: [u8; 4] = reply.get(12..16)?.try_into().ok()?;
    Some(i32::from_le_bytes(bytes))
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

/// AMQP 0-9-1's protocol header. The client speaks first here — contrary to
/// the comment this module used to carry — and the server answers with
/// `Connection.Start` or with the version it would rather use.
const AMQP_HEADER: &[u8] = b"AMQP\x00\x00\x09\x01";

/// Ours, echoed back by anything that understood the question.
const CORRELATION_ID: i32 = 0x0000_7175; // "qu"
const DNS_ID: u16 = 0x7175;

/// An MQTT 3.1.1 `CONNECT` with a clean session, which is the only way to make
/// a broker say anything at all. Clean session is what keeps it read-only: the
/// broker holds no state for us after the socket closes.
fn mqtt_connect() -> Vec<u8> {
    let mut variable = Vec::new();
    variable.extend_from_slice(&4u16.to_be_bytes());
    variable.extend_from_slice(b"MQTT");
    variable.push(0x04); // protocol level 4 — 3.1.1
    variable.push(0x02); // clean session, no will, no credentials
    variable.extend_from_slice(&60u16.to_be_bytes()); // keep alive

    let client_id = b"quarry";
    variable.extend_from_slice(&(client_id.len() as u16).to_be_bytes());
    variable.extend_from_slice(client_id);

    let mut out = vec![0x10]; // CONNECT
    out.extend_from_slice(&remaining_length(variable.len()));
    out.extend_from_slice(&variable);
    out
}

/// MQTT's variable-length integer: seven bits at a time, high bit as the
/// continuation flag.
fn remaining_length(mut n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (n % 128) as u8;
        n /= 128;
        if n > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if n == 0 {
            return out;
        }
    }
}

/// Kafka's `ApiVersions` request, which every broker answers before any
/// authentication precisely so a client can find out what it speaks.
fn kafka_api_versions() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&18i16.to_be_bytes()); // ApiVersions
    body.extend_from_slice(&0i16.to_be_bytes()); // version 0
    body.extend_from_slice(&CORRELATION_ID.to_be_bytes());
    body.extend_from_slice(&(-1i16).to_be_bytes()); // null client id

    let mut out = (body.len() as i32).to_be_bytes().to_vec();
    out.extend_from_slice(&body);
    out
}

/// An `A` query for `localhost` over TCP, which is a resolver's least
/// interesting question and settles whether one is there at all.
fn dns_query() -> Vec<u8> {
    let mut msg = Vec::new();
    msg.extend_from_slice(&DNS_ID.to_be_bytes());
    msg.extend_from_slice(&0x0100u16.to_be_bytes()); // standard query, recursion desired
    msg.extend_from_slice(&1u16.to_be_bytes()); // one question
    msg.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // no answers, authorities, additionals
    msg.push(9);
    msg.extend_from_slice(b"localhost");
    msg.push(0); // end of name
    msg.extend_from_slice(&1u16.to_be_bytes()); // A
    msg.extend_from_slice(&1u16.to_be_bytes()); // IN

    // DNS over TCP is length-prefixed; UDP is not.
    let mut out = (msg.len() as u16).to_be_bytes().to_vec();
    out.extend_from_slice(&msg);
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

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A server that records what it was asked and replies with `reply`.
    fn scripted(reply: Vec<u8>) -> (u16, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 512];
                let n = stream.read(&mut buf).unwrap_or(0);
                let _ = tx.send(buf[..n].to_vec());
                let _ = stream.write_all(&reply);
            }
        });
        (port, rx)
    }

    fn ask(port: u16, name: &str) -> Option<Vec<u8>> {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        run(name, &stream, Duration::from_secs(2))
    }

    /// A broker answers `CONNECT` with `CONNACK` and nothing else does.
    #[test]
    fn mqtt_connects_and_is_answered() {
        let (port, asked) = scripted(vec![0x20, 0x02, 0x00, 0x00]);
        let reply = ask(port, "mqtt").expect("a reply");
        let sent = asked.recv().expect("the request");

        assert_eq!(sent[0], 0x10, "not a CONNECT packet: {sent:02x?}");
        assert_eq!(&sent[2..8], b"\x00\x04MQTT", "wrong protocol name");
        assert_eq!(sent[8], 0x04, "not protocol level 3.1.1");
        assert_eq!(
            sent[9] & 0x02,
            0x02,
            "clean session is what keeps it read-only"
        );
        assert!(confirms("mqtt", &reply));
    }

    /// The client speaks first in AMQP. Either answer identifies the broker.
    #[test]
    fn amqp_sends_the_protocol_header() {
        for reply in [vec![0x01, 0x00, 0x00], b"AMQP\x00\x00\x09\x01".to_vec()] {
            let (port, asked) = scripted(reply.clone());
            let got = ask(port, "amqp").expect("a reply");
            assert_eq!(asked.recv().expect("the request"), AMQP_HEADER);
            assert!(confirms("amqp", &got), "{reply:02x?} was not accepted");
        }
    }

    /// `ApiVersions` is the one thing a broker answers before authentication,
    /// which is exactly why it exists.
    #[test]
    fn kafka_is_asked_which_versions_it_speaks() {
        let mut reply = 6i32.to_be_bytes().to_vec();
        reply.extend_from_slice(&CORRELATION_ID.to_be_bytes());
        reply.extend_from_slice(&0i16.to_be_bytes());
        let (port, asked) = scripted(reply.clone());
        let got = ask(port, "kafka").expect("a reply");

        let sent = asked.recv().expect("the request");
        assert_eq!(
            i32::from_be_bytes(sent[0..4].try_into().unwrap()) as usize,
            sent.len() - 4,
            "the length prefix does not describe the body"
        );
        assert_eq!(i16::from_be_bytes(sent[4..6].try_into().unwrap()), 18);
        assert!(confirms("kafka", &got));
    }

    /// A correlation id that is not ours is somebody else's traffic, not proof.
    #[test]
    fn kafka_does_not_accept_an_unrelated_reply() {
        let mut reply = 6i32.to_be_bytes().to_vec();
        reply.extend_from_slice(&0x1234_5678i32.to_be_bytes());
        assert!(!confirms("kafka", &reply));
    }

    #[test]
    fn dns_asks_for_localhost_over_tcp() {
        let mut msg = DNS_ID.to_be_bytes().to_vec();
        msg.extend_from_slice(&0x8180u16.to_be_bytes()); // response, no error
        msg.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
        let mut reply = (msg.len() as u16).to_be_bytes().to_vec();
        reply.extend_from_slice(&msg);

        let (port, asked) = scripted(reply.clone());
        let got = ask(port, "dns").expect("a reply");

        let sent = asked.recv().expect("the request");
        assert_eq!(
            u16::from_be_bytes(sent[0..2].try_into().unwrap()) as usize,
            sent.len() - 2,
            "DNS over TCP is length-prefixed and this one is not"
        );
        assert!(sent.windows(9).any(|w| w == b"localhost"));
        assert!(confirms("dns", &got));
    }

    /// The question the whole module exists to answer: a port is a convention,
    /// and something else sitting on it must stop being reported as the thing
    /// the number suggests.
    #[test]
    fn a_stranger_on_the_port_is_not_confirmed() {
        let impostor = b"SSH-2.0-OpenSSH_9.6\r\n";
        for name in [
            "redis",
            "memcached",
            "postgres",
            "mongodb",
            "mqtt",
            "kafka",
            "dns",
        ] {
            assert!(
                !confirms(name, impostor),
                "{name} accepted an SSH banner as proof of itself"
            );
        }
    }

    /// And each real answer is accepted.
    #[test]
    fn each_protocol_accepts_its_own_answer() {
        assert!(confirms("redis", b"+PONG\r\n"));
        assert!(confirms("redis", b"-NOAUTH Authentication required.\r\n"));
        assert!(confirms("memcached", b"VERSION 1.6.21\r\n"));
        assert!(confirms("postgres", b"S"));
        assert!(confirms("postgres", b"N"));
        // An older server answers an ErrorResponse rather than S or N.
        let mut err = vec![b'E'];
        err.extend_from_slice(&42i32.to_be_bytes());
        err.extend_from_slice(b"unsupported frontend protocol");
        assert!(confirms("postgres", &err));
        assert!(confirms("http", b"HTTP/1.1 200 OK\r\n"));

        let mut mongo = vec![0u8; 16];
        mongo[12] = 1; // OP_REPLY
        assert!(confirms("mongodb", &mongo));
    }

    /// A protocol quarry has no proof for must not be able to disprove one.
    #[test]
    fn an_unknown_protocol_disproves_nothing() {
        assert!(confirms("something-else", b"anything at all"));
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn memcached_and_mysql_state_their_version_outright() {
        assert_eq!(
            version("memcached", b"VERSION 1.6.21\r\n").as_deref(),
            Some("1.6.21")
        );

        // Packet header, protocol byte, then a NUL-terminated version.
        let mut greeting = vec![0x36, 0x00, 0x00, 0x00, 0x0a];
        greeting.extend_from_slice(b"8.0.36\0");
        greeting.extend_from_slice(&[0x01, 0x02, 0x03]);
        assert_eq!(version("mysql", &greeting).as_deref(), Some("8.0.36"));
    }

    #[test]
    fn a_banner_that_names_itself_is_read_for_one() {
        assert_eq!(
            version("ssh", b"SSH-2.0-OpenSSH_9.6\r\n").as_deref(),
            Some("2.0")
        );
        assert_eq!(
            version("ftp", b"220 ProFTPD 1.3.8 Server\r\n").as_deref(),
            Some("1.3.8")
        );
    }

    /// Guessing a version out of arbitrary bytes would invent one.
    #[test]
    fn nothing_is_invented() {
        assert_eq!(version("redis", b"+PONG\r\n"), None);
        assert_eq!(version("postgres", b"S"), None);
        assert_eq!(version("memcached", b"ERROR\r\n"), None);
        assert_eq!(version("mongodb", &[0u8; 16]), None);
        assert_eq!(version("mysql", b"too short"), None);
    }

    /// A binary reply must not be read as a page of text.
    #[test]
    fn a_version_is_never_longer_than_a_version() {
        let noise: Vec<u8> = std::iter::repeat_n(b'7', 200)
            .chain(b".0".iter().copied())
            .collect();
        let got = version("memcached", &[b"VERSION ".to_vec(), noise].concat());
        assert!(got.is_none(), "{got:?}");
    }
}
