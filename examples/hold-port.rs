//! A process that does nothing but hold a port, for the lifecycle tests.
//!
//! Restarting something has to be tried against a real process: one that owns
//! a real socket, exits on a real SIGTERM, and has a command line and an
//! environment for quarry to read back. A fixture cannot stand in for any of
//! that.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .expect("usage: hold-port <port>");
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .unwrap_or_else(|e| panic!("bind :{port}: {e}"));
    // Announce readiness on stdout, so a test can wait for the socket to exist
    // rather than sleeping and hoping.
    println!("listening on {port}");
    for stream in listener.incoming() {
        drop(stream);
    }
}
