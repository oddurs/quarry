//! Listening sockets from `/proc`, without spawning anything.
//!
//! `lsof` costs about eighty milliseconds and is not installed on a good many
//! container images. The same information is in files: `/proc/net/tcp` and its
//! siblings list every socket with the inode that owns it, and
//! `/proc/<pid>/fd/` is a directory of symlinks reading `socket:[inode]`. The
//! join between them is the whole trick, and it is done once for every socket
//! rather than once per socket — walking the fd table per port is what makes
//! the naive version quadratic.
//!
//! Two Linux-specific things are handled rather than ignored. **Abstract**
//! unix sockets have no filesystem path; their names begin with a NUL and are
//! conventionally printed with a leading `@`. And **network namespaces**: a
//! container's listeners live in their own namespace and do not appear in the
//! host's `/proc/net/tcp` at all, which is why quarry asks the container
//! runtime separately and why it cannot see into a namespace it is not in.

use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use crate::model::{Listener, Transport};
use crate::source::{RawSocket, SocketSource, SourceError};

/// `TCP_LISTEN`, the only state that means somebody is waiting for callers.
const TCP_LISTEN: &str = "0A";

pub struct Proc {
    root: PathBuf,
}

impl Default for Proc {
    fn default() -> Self {
        Proc::at("/proc")
    }
}

impl Proc {
    pub fn at(root: impl Into<PathBuf>) -> Proc {
        Proc { root: root.into() }
    }
}

/// Whether this machine has a `/proc` worth reading.
pub fn available() -> bool {
    Path::new("/proc/net/tcp").exists() && Path::new("/proc/self/fd").is_dir()
}

impl SocketSource for Proc {
    fn listening(&mut self) -> Result<Vec<RawSocket>, SourceError> {
        let read = |name: &str| fs::read_to_string(self.root.join(name)).unwrap_or_default();

        // Every socket, each carrying the inode that will name its owner.
        let mut bound: Vec<(u64, Listener)> = Vec::new();
        bound.extend(parse_net(&read("net/tcp"), Transport::Tcp, false));
        bound.extend(parse_net(&read("net/tcp6"), Transport::Tcp, true));
        bound.extend(parse_net(&read("net/udp"), Transport::Udp, false));
        bound.extend(parse_net(&read("net/udp6"), Transport::Udp, true));
        bound.extend(parse_unix(&read("net/unix")));

        if bound.is_empty() && !available() {
            return Err(SourceError {
                source: "/proc",
                detail: "no /proc on this machine".into(),
                transient: false,
            });
        }

        let owners = self.owners();
        Ok(bound
            .into_iter()
            .filter_map(|(inode, listener)| {
                let (pid, command, user) = owners.get(&inode)?.clone();
                Some(RawSocket {
                    pid,
                    command,
                    user,
                    listener,
                })
            })
            .collect())
    }

    fn describe(&self) -> String {
        "/proc (native)".to_string()
    }
}

impl Proc {
    /// inode → the process holding it.
    ///
    /// One walk of every process's fd table. Doing it the other way round —
    /// searching the fd tables for each socket — is the same work multiplied
    /// by the number of ports.
    fn owners(&self) -> HashMap<u64, (u32, String, String)> {
        let mut owners = HashMap::new();
        let Ok(entries) = fs::read_dir(&self.root) else {
            return owners;
        };
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let dir = entry.path();
            let command = fs::read_to_string(dir.join("comm"))
                .map(|c| c.trim().to_string())
                .unwrap_or_default();
            let user = uid_of(&dir).map(|u| u.to_string()).unwrap_or_default();

            // A process that exits mid-walk takes its fd directory with it,
            // which is ordinary rather than an error.
            let Ok(fds) = fs::read_dir(dir.join("fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                let Ok(target) = fs::read_link(fd.path()) else {
                    continue;
                };
                if let Some(inode) = socket_inode(&target.to_string_lossy()) {
                    owners.insert(inode, (pid, command.clone(), user.clone()));
                }
            }
        }
        owners
    }
}

/// `socket:[12345]` — the only fd target worth anything here.
fn socket_inode(target: &str) -> Option<u64> {
    target
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// The owning uid, from the `Uid:` line of `/proc/<pid>/status`.
fn uid_of(dir: &Path) -> Option<u32> {
    let status = fs::read_to_string(dir.join("status")).ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// `/proc/net/tcp`, `tcp6`, `udp` and `udp6`, which share a layout.
///
/// TCP listeners are state `0A`. UDP has no listen state at all, so a socket
/// with no peer is the closest thing to one — which is the same judgement
/// quarry already makes when it calls a bound UDP port `bound` rather than
/// claiming a check it did not do.
pub fn parse_net(text: &str, transport: Transport, v6: bool) -> Vec<(u64, Listener)> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        // sl, local, remote, st, …, inode is the tenth field.
        let (Some(local), Some(remote), Some(state), Some(inode)) =
            (f.get(1), f.get(2), f.get(3), f.get(9))
        else {
            continue;
        };
        let listening = match transport {
            Transport::Tcp => *state == TCP_LISTEN,
            _ => remote.ends_with(":0000"),
        };
        if !listening {
            continue;
        }
        let (Some((addr, port)), Ok(inode)) = (endpoint(local, v6), inode.parse::<u64>()) else {
            continue;
        };
        // Inode zero is a socket with no owning process — a TIME_WAIT entry or
        // a kernel-internal one — and joining on it would attribute every such
        // socket to whoever happens to hash there.
        if inode == 0 {
            continue;
        }
        out.push((
            inode,
            Listener {
                transport,
                addr,
                port,
                path: None,
                wildcard: addr.is_unspecified(),
            },
        ));
    }
    out
}

/// `0100007F:1F90` — the address in the host's word order, the port in the
/// network's. Both hex, and the v4 address is byte-reversed because it is a
/// little-endian `u32` printed as one.
fn endpoint(field: &str, v6: bool) -> Option<(IpAddr, u16)> {
    let (addr, port) = field.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    if v6 {
        if addr.len() != 32 {
            return None;
        }
        // Four little-endian 32-bit words, in order.
        let mut octets = [0u8; 16];
        for (i, chunk) in addr.as_bytes().chunks(8).enumerate() {
            let word = u32::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
            octets[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        Some((IpAddr::V6(Ipv6Addr::from(octets)), port))
    } else {
        if addr.len() != 8 {
            return None;
        }
        let raw = u32::from_str_radix(addr, 16).ok()?;
        Some((IpAddr::V4(Ipv4Addr::from(raw.to_le_bytes())), port))
    }
}

/// `/proc/net/unix`. A listening socket is one with the `ACC` flag set —
/// `00010000` — which is what distinguishes a server's socket from the
/// connections to it.
pub fn parse_unix(text: &str) -> Vec<(u64, Listener)> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        // Num RefCount Protocol Flags Type St Inode Path
        let (Some(flags), Some(inode)) = (f.get(3), f.get(6)) else {
            continue;
        };
        let Ok(flags) = u32::from_str_radix(flags, 16) else {
            continue;
        };
        if flags & 0x0001_0000 == 0 {
            continue;
        }
        let Ok(inode) = inode.parse::<u64>() else {
            continue;
        };
        // An abstract socket has no filesystem path; the kernel prints its
        // leading NUL as `@`. Keeping the `@` is how it is written everywhere
        // else, and it is not a path anyone can open.
        let Some(path) = f.get(7) else { continue };
        out.push((
            inode,
            Listener {
                transport: Transport::Unix,
                addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                port: 0,
                path: Some(PathBuf::from(*path)),
                wildcard: false,
            },
        ));
    }
    out
}
