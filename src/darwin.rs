//! Listening sockets straight from the kernel, without `lsof`.
//!
//! `lsof` is 98% of the cost of a scan — 58ms of about 59 — because it walks
//! every file descriptor of every process and formats the result as text for us
//! to parse back. This asks the same questions of the same interface `lsof`
//! uses, `libproc`, and skips both the process spawn and the round trip through
//! text.
//!
//! It is also the more durable path: no external binary to be missing, renamed,
//! or replaced by something that prints a different format. `lsof` stays as the
//! fallback for the cases this cannot cover.
//!
//! # Layout
//!
//! The structures below mirror `<sys/proc_info.h>`. Getting one wrong would
//! read garbage rather than fail, so two things guard against that: a size
//! check against what the kernel actually writes, and a test comparing this
//! source's output against `lsof`'s on the machine running the tests.

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::model::Listener;
use crate::source::{RawSocket, SocketSource, SourceError};

const PROC_ALL_PIDS: u32 = 1;
/// A flavour for `proc_pidfdinfo`, *not* for `proc_pidinfo` — both take a
/// flavour argument and 3 means something different to each. Passing it to the
/// wrong one returns `proc_bsdinfo` and reads as an empty machine.
const PROC_PIDFDSOCKETINFO: libc::c_int = 3;

const SOCKINFO_IN: libc::c_int = 1;
const SOCKINFO_TCP: libc::c_int = 2;
const SOCKINFO_UN: libc::c_int = 3;
const SOCK_DGRAM: libc::c_int = 2;
/// `sizeof(sockaddr_un)`; the path is the tail of it.
const UN_PATH_OFFSET: usize = 2;
const TSI_S_LISTEN: libc::c_int = 1;
const INI_IPV4: u8 = 0x1;
const INI_IPV6: u8 = 0x2;
const TSI_T_NTIMERS: usize = 4;
const MAXCOMLEN: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct VinfoStat {
    vst_dev: u32,
    vst_mode: u16,
    vst_nlink: u16,
    vst_ino: u64,
    vst_uid: libc::uid_t,
    vst_gid: libc::gid_t,
    vst_atime: i64,
    vst_atimensec: i64,
    vst_mtime: i64,
    vst_mtimensec: i64,
    vst_ctime: i64,
    vst_ctimensec: i64,
    vst_birthtime: i64,
    vst_birthtimensec: i64,
    vst_size: i64,
    vst_blocks: i64,
    vst_blksize: i32,
    vst_flags: u32,
    vst_gen: u32,
    vst_rdev: u32,
    vst_qspare: [i64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SockbufInfo {
    sbi_cc: u32,
    sbi_hiwat: u32,
    sbi_mbcnt: u32,
    sbi_mbmax: u32,
    sbi_lowat: u32,
    sbi_flags: libc::c_short,
    sbi_timeo: libc::c_short,
}

/// `in_sockinfo`. The union members are both 16 bytes, so a plain array stands
/// in for them: IPv4 addresses live in the last four bytes, which is what
/// `in4in6_addr` describes.
#[repr(C)]
#[derive(Clone, Copy)]
struct InSockinfo {
    insi_fport: libc::c_int,
    insi_lport: libc::c_int,
    insi_gencnt: u64,
    insi_flags: u32,
    insi_flow: u32,
    insi_vflag: u8,
    insi_ip_ttl: u8,
    rfu_1: u32,
    insi_faddr: [u8; 16],
    insi_laddr: [u8; 16],
    insi_v4: InsiV4,
    insi_v6: InsiV6,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InsiV4 {
    in4_tos: libc::c_uchar,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InsiV6 {
    in6_hlim: u8,
    in6_cksum: libc::c_int,
    in6_ifindex: libc::c_ushort,
    in6_hops: libc::c_short,
}

/// `un_sockinfo`: two addresses, of which only the local one matters.
///
/// Each address is a `sockaddr_un` inside a 256-byte union, not a bare
/// `sockaddr_un` — which is why this is 528 bytes rather than the 228 a naive
/// reading of the header gives.
#[repr(C)]
#[derive(Clone, Copy)]
struct UnSockinfo {
    unsi_conn_so: u64,
    unsi_conn_pcb: u64,
    unsi_addr: [u8; 256],
    unsi_caddr: [u8; 256],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TcpSockinfo {
    tcpsi_ini: InSockinfo,
    tcpsi_state: libc::c_int,
    tcpsi_timer: [libc::c_int; TSI_T_NTIMERS],
    tcpsi_mss: libc::c_int,
    tcpsi_flags: u32,
    rfu_1: u32,
    tcpsi_tp: u64,
}

/// `socket_info`, truncated after the union member we read.
///
/// `soi_proto` is a union whose largest member is not `tcp_sockinfo`, so this
/// struct is smaller than the kernel's. That is fine in one direction only:
/// every field we read comes before the end of `pri_tcp`, and the buffer handed
/// to the kernel is sized from the kernel's own constant, not from this.
#[repr(C)]
#[derive(Clone, Copy)]
struct SocketInfo {
    soi_stat: VinfoStat,
    soi_so: u64,
    soi_pcb: u64,
    soi_type: libc::c_int,
    soi_protocol: libc::c_int,
    soi_family: libc::c_int,
    soi_options: libc::c_short,
    soi_linger: libc::c_short,
    soi_state: libc::c_short,
    soi_qlen: libc::c_short,
    soi_incqlen: libc::c_short,
    soi_qlimit: libc::c_short,
    soi_timeo: libc::c_short,
    soi_error: libc::c_ushort,
    soi_oobmark: u32,
    soi_rcv: SockbufInfo,
    soi_snd: SockbufInfo,
    soi_kind: libc::c_int,
    rfu_1: u32,
    /// The `soi_proto` union. Read as whichever member `soi_kind` names; the
    /// buffer handed to the kernel is sized from its constant, not from this,
    /// so a member smaller than the real union is safe to read.
    soi_proto: SoiProto,
}

#[repr(C)]
#[derive(Clone, Copy)]
union SoiProto {
    tcp: TcpSockinfo,
    inet: InSockinfo,
    un: UnSockinfo,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcFileinfo {
    fi_openflags: u32,
    fi_status: u32,
    fi_offset: libc::off_t,
    fi_type: i32,
    fi_guardflags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SocketFdinfo {
    pfi: ProcFileinfo,
    psi: SocketInfo,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BsdInfo {
    pbi_flags: u32,
    pbi_status: u32,
    pbi_xstatus: u32,
    pbi_pid: u32,
    pbi_ppid: u32,
    pbi_uid: libc::uid_t,
    pbi_gid: libc::gid_t,
    pbi_ruid: libc::uid_t,
    pbi_rgid: libc::gid_t,
    pbi_svuid: libc::uid_t,
    pbi_svgid: libc::gid_t,
    rfu_1: u32,
    pbi_comm: [libc::c_char; MAXCOMLEN],
    pbi_name: [libc::c_char; 2 * MAXCOMLEN],
    pbi_nfiles: u32,
    pbi_pgid: u32,
    pbi_pjobc: u32,
    e_tdev: u32,
    e_tpgid: u32,
    pbi_nice: i32,
    pbi_start_tvsec: u64,
    pbi_start_tvusec: u64,
}

const PROC_PIDTBSDINFO: libc::c_int = 3;

pub struct Native;

impl SocketSource for Native {
    fn listening(&mut self) -> Result<Vec<RawSocket>, SourceError> {
        listening_sockets().map_err(|detail| SourceError {
            source: "libproc",
            detail,
            transient: false,
        })
    }

    fn describe(&self) -> String {
        "libproc (native)".to_string()
    }
}

/// Whether this platform can answer without `lsof`.
pub fn available() -> bool {
    // A single cheap call: if the kernel will not enumerate pids for us, it
    // will not answer anything else either.
    all_pids().map(|p| !p.is_empty()).unwrap_or(false)
}

fn all_pids() -> Result<Vec<i32>, String> {
    // Ask for the size first; the count moves between the two calls, so the
    // buffer is oversized deliberately rather than exactly.
    let needed = unsafe { libc::proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return Err("proc_listpids reported no processes".into());
    }
    let capacity = (needed as usize / size_of::<i32>()) + 64;
    let mut pids: Vec<i32> = vec![0; capacity];
    let written = unsafe {
        libc::proc_listpids(
            PROC_ALL_PIDS,
            0,
            pids.as_mut_ptr() as *mut libc::c_void,
            (capacity * size_of::<i32>()) as libc::c_int,
        )
    };
    if written <= 0 {
        return Err("proc_listpids failed".into());
    }
    pids.truncate(written as usize / size_of::<i32>());
    pids.retain(|p| *p > 0);
    Ok(pids)
}

fn listening_sockets() -> Result<Vec<RawSocket>, String> {
    let pids = all_pids()?;
    let mut out = Vec::new();
    // Reused across every process rather than reallocated per process; a busy
    // machine has hundreds of processes with dozens of descriptors each.
    let mut fds: Vec<libc::proc_fdinfo> = Vec::with_capacity(256);

    for pid in pids {
        if !list_fds(pid, &mut fds) {
            continue; // Another user's process, or one that exited mid-scan.
        }
        let mut command: Option<String> = None;
        let mut user: Option<String> = None;

        for fd in fds.iter().filter(|f| f.proc_fdtype == PROX_FDTYPE_SOCKET) {
            let Some(listener) = listening_addr(pid, fd.proc_fd) else {
                continue;
            };
            // Only worth asking once, and only for a process that has one.
            if command.is_none() {
                let (name, uid) = bsd_info(pid);
                command = Some(name);
                user = Some(uid);
            }
            out.push(RawSocket {
                pid: pid as u32,
                command: command.clone().unwrap_or_default(),
                user: user.clone().unwrap_or_default(),
                listener,
            });
        }
    }
    out.sort_by_key(|s| (s.pid, s.listener.port));
    Ok(out)
}

const PROX_FDTYPE_SOCKET: u32 = 2;

fn list_fds(pid: i32, buf: &mut Vec<libc::proc_fdinfo>) -> bool {
    let needed =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return false;
    }
    let count = needed as usize / size_of::<libc::proc_fdinfo>() + 16;
    if buf.len() < count {
        buf.resize(
            count,
            libc::proc_fdinfo {
                proc_fd: 0,
                proc_fdtype: 0,
            },
        );
    }
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            buf.as_mut_ptr() as *mut libc::c_void,
            (buf.len() * size_of::<libc::proc_fdinfo>()) as libc::c_int,
        )
    };
    if written <= 0 {
        return false;
    }
    buf.truncate(written as usize / size_of::<libc::proc_fdinfo>());
    true
}

/// The local endpoint of `fd`, if it is one worth showing.
///
/// Three cases: a TCP socket in LISTEN, a bound UDP socket, and a unix socket
/// with a path. Everything else — an established connection, an anonymous
/// socketpair — is not something anybody is serving on.
fn listening_addr(pid: i32, fd: i32) -> Option<Listener> {
    // Sized generously: the kernel's `socket_fdinfo` is larger than the
    // truncated struct above, and it decides how much to write.
    let mut buf = [0u8; 2048];
    let written = unsafe {
        libc::proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDSOCKETINFO,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as libc::c_int,
        )
    };
    // A short write means the kernel's layout is not the one modelled here;
    // reading it would invent addresses rather than fail, so refuse.
    if written < size_of::<SocketFdinfo>() as libc::c_int {
        return None;
    }

    // SAFETY: the kernel wrote at least `size_of::<SocketFdinfo>()` bytes of a
    // structure this one is a prefix of, into a correctly aligned buffer.
    let info: SocketFdinfo =
        unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const SocketFdinfo) };

    match info.psi.soi_kind {
        // SAFETY: `soi_kind` names the union member the kernel filled in.
        SOCKINFO_TCP => {
            let tcp = unsafe { info.psi.soi_proto.tcp };
            if tcp.tcpsi_state != TSI_S_LISTEN {
                return None;
            }
            endpoint(&tcp.tcpsi_ini).map(|(addr, port)| Listener::tcp(addr, port))
        }
        SOCKINFO_IN if info.psi.soi_type == SOCK_DGRAM => {
            // UDP has no listening state. A socket bound to a port with no peer
            // is the closest honest equivalent, and it is what `lsof -iUDP`
            // shows.
            let ini = unsafe { info.psi.soi_proto.inet };
            if ini.insi_fport != 0 {
                return None; // connected, so not serving
            }
            endpoint(&ini).map(|(addr, port)| Listener::udp(addr, port))
        }
        SOCKINFO_UN => {
            let un = unsafe { info.psi.soi_proto.un };
            unix_path(&un.unsi_addr).map(Listener::unix)
        }
        _ => None,
    }
}

/// The local address and port from an `in_sockinfo`, in host order.
fn endpoint(ini: &InSockinfo) -> Option<(IpAddr, u16)> {
    let port = u16::from_be(ini.insi_lport as u16);
    if port == 0 {
        return None;
    }
    let addr = if ini.insi_vflag & INI_IPV4 != 0 {
        // An IPv4 address sits in the last four bytes of the 16-byte union.
        let b = &ini.insi_laddr[12..16];
        IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3]))
    } else if ini.insi_vflag & INI_IPV6 != 0 {
        let mut octets = [0u8; 16];
        octets.copy_from_slice(&ini.insi_laddr);
        IpAddr::V6(Ipv6Addr::from(octets))
    } else {
        return None;
    };
    Some((addr, port))
}

/// The path out of a `sockaddr_un`: a length byte, a family byte, then the
/// path. An unnamed socket — one end of a `socketpair` — has none, and is not
/// something anybody is serving on.
fn unix_path(addr: &[u8; 256]) -> Option<std::path::PathBuf> {
    let bytes = &addr[UN_PATH_OFFSET..];
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes[..end]);
    Some(std::path::PathBuf::from(text.as_ref()))
}

/// The process's short name and login name, as `lsof` would report them.
fn bsd_info(pid: i32) -> (String, String) {
    let mut info = BsdInfo::default();
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            PROC_PIDTBSDINFO,
            0,
            &mut info as *mut BsdInfo as *mut libc::c_void,
            size_of::<BsdInfo>() as libc::c_int,
        )
    };
    if written < size_of::<BsdInfo>() as libc::c_int {
        return (String::new(), String::new());
    }
    // `pbi_name` is the full name where the kernel has one, `pbi_comm` the
    // truncated form. lsof prints the longer of the two.
    let name = c_string(&info.pbi_name);
    let comm = c_string(&info.pbi_comm);
    let command = if name.is_empty() { comm } else { name };
    (command, user_name(info.pbi_uid))
}

fn c_string(bytes: &[libc::c_char]) -> String {
    let end = bytes.iter().position(|c| *c == 0).unwrap_or(bytes.len());
    let slice: &[u8] = unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u8, end) };
    String::from_utf8_lossy(slice).to_string()
}

/// uid → login name, cached: a scan sees the same handful of uids repeatedly
/// and `getpwuid` is not free.
fn user_name(uid: libc::uid_t) -> String {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(map) = cache.lock()
        && let Some(name) = map.get(&uid)
    {
        return name.clone();
    }

    // SAFETY: a lookup returning a pointer into a static buffer, read before
    // any other call to the same function can replace it.
    let name = unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            uid.to_string()
        } else {
            let cstr = std::ffi::CStr::from_ptr((*pw).pw_name);
            cstr.to_string_lossy().to_string()
        }
    };
    if let Ok(mut map) = cache.lock() {
        map.insert(uid, name.clone());
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Transport;
    use crate::source::SocketSource;

    /// Sizes and offsets taken from a C program compiled against the SDK's
    /// `<sys/proc_info.h>`. If any of these drift, every field after the
    /// mismatch reads plausible garbage rather than failing, so they are
    /// asserted rather than assumed.
    #[test]
    fn the_kernel_agrees_with_our_idea_of_its_layout() {
        assert_eq!(size_of::<VinfoStat>(), 136, "vinfo_stat");
        assert_eq!(size_of::<SockbufInfo>(), 24, "sockbuf_info");
        assert_eq!(size_of::<InSockinfo>(), 80, "in_sockinfo");
        assert_eq!(size_of::<TcpSockinfo>(), 120, "tcp_sockinfo");
        assert_eq!(size_of::<ProcFileinfo>(), 24, "proc_fileinfo");
        assert_eq!(size_of::<BsdInfo>(), 136, "proc_bsdinfo");
        // Two handles plus two 256-byte address unions.
        assert_eq!(size_of::<UnSockinfo>(), 528, "un_sockinfo");
        assert_eq!(size_of::<SoiProto>(), 528, "the soi_proto union");

        let base = std::ptr::null::<SocketFdinfo>();
        // SAFETY: offset arithmetic on a null pointer, never dereferenced.
        let offset = |field: *const u8| unsafe { field.offset_from(base as *const u8) as usize };
        unsafe {
            assert_eq!(offset(&raw const (*base).psi as *const u8), 24, "psi");
            assert_eq!(
                offset(&raw const (*base).psi.soi_family as *const u8),
                184,
                "soi_family"
            );
            assert_eq!(
                offset(&raw const (*base).psi.soi_kind as *const u8),
                256,
                "soi_kind"
            );
            assert_eq!(
                offset(&raw const (*base).psi.soi_proto as *const u8),
                264,
                "soi_proto"
            );
            let tcp = &raw const (*base).psi.soi_proto.tcp;
            assert_eq!(
                offset(&raw const (*tcp).tcpsi_ini.insi_lport as *const u8),
                268,
                "insi_lport"
            );
            assert_eq!(
                offset(&raw const (*tcp).tcpsi_ini.insi_vflag),
                288,
                "insi_vflag"
            );
            assert_eq!(
                offset(&raw const (*tcp).tcpsi_ini.insi_laddr as *const u8),
                312,
                "insi_laddr"
            );
            assert_eq!(
                offset(&raw const (*tcp).tcpsi_state as *const u8),
                344,
                "tcpsi_state"
            );
        }
    }

    #[test]
    fn it_sees_a_unix_socket_this_test_is_listening_on() {
        let dir = std::env::temp_dir().join(format!("quarry-unix-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");

        let found = Native.listening().expect("native source works");
        let mine = found.iter().find(|s| {
            s.pid == std::process::id() && s.listener.path.as_deref() == Some(path.as_path())
        });
        assert!(
            mine.is_some(),
            "a unix socket this process is listening on was not found; \
             saw {:?}",
            found
                .iter()
                .filter(|s| s.pid == std::process::id() && s.listener.is_unix())
                .map(|s| s.listener.path.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(mine.expect("checked").listener.transport, Transport::Unix);

        drop(listener);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn it_sees_a_bound_udp_socket() {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
        let port = socket.local_addr().expect("addr").port();

        let found = Native.listening().expect("native source works");
        let mine = found.iter().find(|s| {
            s.pid == std::process::id()
                && s.listener.port == port
                && s.listener.transport == Transport::Udp
        });
        assert!(mine.is_some(), "a bound UDP socket was not found");
    }

    #[test]
    fn it_finds_the_socket_this_test_is_listening_on() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let found = Native.listening().expect("native source works");
        let mine = found
            .iter()
            .find(|s| s.pid == std::process::id() && s.listener.port == port);
        assert!(
            mine.is_some(),
            "the native source missed a socket this very process is listening on"
        );
        let mine = mine.expect("checked above");
        assert_eq!(mine.listener.addr.to_string(), "127.0.0.1");
        assert!(!mine.command.is_empty(), "no process name");
        assert!(!mine.user.is_empty(), "no user name");
    }

    #[test]
    fn it_ignores_sockets_that_are_not_listening() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        // A connected pair on the same process: neither end is in LISTEN.
        let client = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let (server, _) = listener.accept().expect("accept");

        let found = Native.listening().expect("native source works");
        let mine: Vec<u16> = found
            .iter()
            .filter(|s| s.pid == std::process::id())
            .map(|s| s.listener.port)
            .collect();
        assert!(mine.contains(&port), "the listener itself should be there");
        let client_port = client.local_addr().expect("addr").port();
        assert!(
            !mine.contains(&client_port),
            "an established connection is not a listening socket"
        );
        drop((client, server, listener));
    }

    /// The parity check that makes the layout above trustworthy.
    ///
    /// Compared on TCP only: `lsof -iTCP` is what the fallback reports, and the
    /// native source sees more than that by design.
    #[test]
    fn it_agrees_with_lsof_about_tcp() {
        let mut lsof = crate::lsof::Lsof;
        let Ok(theirs) = lsof.listening() else {
            eprintln!("lsof unavailable; skipping the parity check");
            return;
        };
        let ours = Native.listening().expect("native source works");

        // This test binary binds and drops sockets constantly in other tests,
        // and the two samples are taken milliseconds apart. Its own churn is
        // not a disagreement about layout, which is what this is checking.
        let mine = std::process::id();
        let key = |s: &RawSocket| (s.pid, s.listener.port);
        let mut theirs: Vec<_> = theirs.iter().filter(|s| s.pid != mine).map(key).collect();
        let mut ours_keys: Vec<_> = ours
            .iter()
            .filter(|s| s.listener.transport == Transport::Tcp && s.pid != mine)
            .map(key)
            .collect();
        theirs.sort_unstable();
        theirs.dedup();
        ours_keys.sort_unstable();
        ours_keys.dedup();

        // The two run milliseconds apart, so a socket may legitimately appear
        // or vanish between them. What must not happen is systematic
        // disagreement, which is what a wrong struct layout produces.
        let missing: Vec<_> = theirs.iter().filter(|k| !ours_keys.contains(k)).collect();
        let extra: Vec<_> = ours_keys.iter().filter(|k| !theirs.contains(k)).collect();
        let total = theirs.len().max(1);
        assert!(
            missing.len() * 5 < total,
            "the native source missed {}/{} of what lsof found: {missing:?}",
            missing.len(),
            total
        );
        assert!(
            extra.len() * 5 < total,
            "the native source invented {} TCP sockets lsof did not see: {extra:?}",
            extra.len()
        );
    }

    /// The surface this was built to reach: on a developer machine there are
    /// far more unix sockets than TCP ports, and quarry could see none of them.
    #[test]
    fn it_sees_more_than_tcp() {
        let found = Native.listening().expect("native source works");
        let unix = found.iter().filter(|s| s.listener.is_unix()).count();
        assert!(
            unix > 0,
            "no unix sockets at all, which is not plausible on a running machine"
        );
        for socket in found.iter().filter(|s| s.listener.is_unix()) {
            let path = socket
                .listener
                .path
                .as_ref()
                .expect("a unix socket has a path");
            assert!(
                !path.as_os_str().is_empty(),
                "an unnamed socket is not something anybody is serving on"
            );
        }
    }
}
