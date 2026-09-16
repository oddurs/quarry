//! An address something is listening on.

use std::net::IpAddr;
use std::path::PathBuf;

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

    /// What goes in a list column, where the width belongs to the table rather
    /// than to this value.
    ///
    /// Always the socket's own name, never the path to it. `label` keeps the
    /// directory while it fits, which is right in a detail pane and wrong in a
    /// column: `/tmp/cc-socks/52425.sock` is twenty-four characters of which
    /// five distinguish it from the next one.
    pub fn column(&self) -> String {
        match &self.path {
            Some(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string()),
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
