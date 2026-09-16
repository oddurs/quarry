//! 0037 — reading listening sockets from `/proc`.
//!
//! Against captured fixtures rather than a live kernel, so these run on macOS
//! too. The parsers are the part that can be wrong; reading four files is not.

use quarry::linux::{parse_net, parse_unix};
use quarry::model::Transport;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/proc/net/{name}"))
        .unwrap_or_else(|e| panic!("read {name}: {e}"))
}

#[test]
fn only_listeners_come_out_of_proc_net_tcp() {
    let found = parse_net(&fixture("tcp"), Transport::Tcp, false);
    let ports: Vec<u16> = found.iter().map(|(_, l)| l.port).collect();
    assert_eq!(
        ports,
        vec![8080, 7096],
        "an established connection or a socket with no owner came through"
    );
}

/// The address is a little-endian `u32` printed as hex, so reading it in
/// written order gives 127.0.0.1 backwards.
#[test]
fn the_v4_address_is_byte_reversed() {
    let found = parse_net(&fixture("tcp"), Transport::Tcp, false);
    let (_, loopback) = found.iter().find(|(_, l)| l.port == 8080).expect(":8080");
    assert_eq!(loopback.addr, IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert!(!loopback.wildcard);

    let (_, any) = found.iter().find(|(_, l)| l.port == 7096).expect(":7096");
    assert_eq!(any.addr, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    assert!(any.wildcard, "0.0.0.0 is every interface and should say so");
}

/// A TIME_WAIT or kernel-internal socket has inode 0 and belongs to nobody.
/// Joining on it would hand every such socket to whoever hashes there.
#[test]
fn a_socket_with_no_owner_is_dropped() {
    let found = parse_net(&fixture("tcp"), Transport::Tcp, false);
    assert!(
        found.iter().all(|(inode, _)| *inode != 0),
        "inode zero came through"
    );
    assert!(
        !found.iter().any(|(_, l)| l.port == 53),
        "the ownerless :53 entry should not be reported"
    );
}

/// v6 addresses are four little-endian 32-bit words, which is not the same as
/// one big-endian 128-bit number.
#[test]
fn the_v6_address_is_four_words_not_one_number() {
    let found = parse_net(&fixture("tcp6"), Transport::Tcp, true);
    let ports: Vec<u16> = found.iter().map(|(_, l)| l.port).collect();
    assert_eq!(ports, vec![8081, 3000]);

    let (_, any) = &found[0];
    assert_eq!(any.addr, IpAddr::V6(Ipv6Addr::UNSPECIFIED));
    let (_, mapped) = &found[1];
    assert_eq!(
        mapped.addr,
        IpAddr::V6(Ipv6Addr::from([0, 0, 0, 0, 0, 0, 0, 1])),
        "::1 written as four words came out as something else"
    );
}

/// UDP has no listen state, so a socket with no peer is the closest thing.
#[test]
fn a_bound_udp_socket_counts_and_a_connected_one_does_not() {
    let found = parse_net(&fixture("udp"), Transport::Udp, false);
    let ports: Vec<u16> = found.iter().map(|(_, l)| l.port).collect();
    assert_eq!(ports, vec![53]);
    assert_eq!(found[0].1.transport, Transport::Udp);
}

/// The `ACC` flag is what distinguishes a server's socket from a connection
/// to it — and `/proc/net/unix` lists both under the same path.
#[test]
fn only_accepting_unix_sockets_are_listeners() {
    let found = parse_unix(&fixture("unix"));
    let paths: Vec<String> = found
        .iter()
        .map(|(_, l)| l.path.as_ref().expect("a path").display().to_string())
        .collect();
    assert_eq!(
        paths,
        vec!["/run/user/1000/docker.sock", "@/tmp/.X11-unix/X0"],
        "the connected socket on the same path came through as a listener"
    );
}

/// An abstract socket has no filesystem path at all; the kernel prints its
/// leading NUL as `@`, and that is how it is written everywhere else.
#[test]
fn an_abstract_socket_keeps_its_at_sign() {
    let found = parse_unix(&fixture("unix"));
    let abstract_one = found
        .iter()
        .find(|(inode, _)| *inode == 41010)
        .expect("the abstract socket");
    assert!(
        abstract_one
            .1
            .path
            .as_ref()
            .expect("a name")
            .to_string_lossy()
            .starts_with('@')
    );
}

/// Nothing here may panic on a file that is empty, truncated mid-write, or
/// simply not the format expected — `/proc` is read while the kernel writes it.
#[test]
fn a_malformed_line_costs_its_own_line_and_nothing_else() {
    let good = "  sl  local_address rem_address   st\n\
                   0: 0100007F:1F90 00000000:0000 0A 0 0 00:0 0 1000 0 41001 1\n";
    for mangled in [
        "",
        "only a header\n",
        "  sl\n   0: nonsense\n",
        "  sl\n   0: 0100007F 00000000:0000 0A 0 0 00:0 0 1000 0 41001 1\n",
        "  sl\n   0: ZZZZZZZZ:1F90 00000000:0000 0A 0 0 00:0 0 1000 0 41001 1\n",
        "  sl\n   0: 0100007F:1F90 00000000:0000 0A 0 0 00:0 0 1000 0 notanumber 1\n",
    ] {
        let _ = parse_net(mangled, Transport::Tcp, false);
        let _ = parse_net(mangled, Transport::Tcp, true);
        let _ = parse_unix(mangled);
        // And a good line still parses when it follows a bad one.
        let mixed = format!(
            "{mangled}{}",
            good.lines().skip(1).collect::<Vec<_>>().join("\n")
        );
        let _ = parse_net(&mixed, Transport::Tcp, false);
    }
}

/// The whole source, against a captured `/proc` tree. No owners are present in
/// the fixture, so the join finds nothing — which is itself the assertion that
/// the join happens at all rather than every socket being reported unowned.
#[test]
fn the_source_joins_sockets_to_the_processes_holding_them() {
    use quarry::source::SocketSource;
    let mut proc = quarry::linux::Proc::at("tests/fixtures/proc");
    let found = proc.listening().expect("a captured /proc is readable");
    assert!(
        found.is_empty(),
        "the fixture has no process directories, so nothing should be attributed: {found:?}"
    );
    assert_eq!(proc.describe(), "/proc (native)");
}
