---
id: 37
title: Read listening sockets from /proc on Linux
type: feature
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
depends_on:
- 28
created: 2026-09-08
updated: 2026-09-15
priority: p1
area: discovery
effort: m
---

## Problem

macOS asks the kernel directly and a scan costs 5ms. Linux falls back to `lsof`
and pays the 80ms — and depends on a binary that is not installed by default on
a good many container images and minimal distributions.

Linux is where most people run the things this milestone is about.

## Proposal

The same information is in `/proc`, in files, with no process to spawn:

- `/proc/net/tcp`, `/proc/net/tcp6` — listeners are state `0A`, with an inode
- `/proc/net/udp`, `/proc/net/udp6` — for 0032
- `/proc/net/unix` — for 0028
- `/proc/<pid>/fd/*` — symlinks reading `socket:[inode]`, which is the join

Two Linux-specific things need handling rather than ignoring:

- **Abstract sockets**, whose names begin with a NUL and print as `@name`.
- **Network namespaces.** A container's listeners are in its own namespace and
  do not appear in the host's `/proc/net/tcp` at all — which is a large part of
  why 0030 exists, and worth saying plainly in the docs rather than leaving
  people to wonder where their container went.

## Acceptance criteria

- [x] a native Linux source, `lsof` still the fallback
- [x] the inode join is done once for all sockets, not once per socket
- [x] parses against captured `/proc/net/*` fixtures, so the tests run anywhere
- [x] `--doctor` reports which source is live, as it does on macOS
- [x] measured against `lsof` on Linux, with the ratio asserted as on macOS

## 2026-09-15

The module is deliberately not gated to Linux. Only choosing the source is platform-specific; the parsers are pure functions over text, and a captured /proc tree is readable from anywhere — so Proc::at(dir) points at a fixture and the whole thing is tested on macOS as well, which is where it was written.

Three things the fixtures exist to pin down. The v4 address is a little-endian u32 printed as hex, so reading it in written order gives 127.0.0.1 backwards. The v6 address is four little-endian words, not one big-endian number. And inode 0 is a socket with no owning process — TIME_WAIT, or kernel-internal — which would otherwise be joined to whichever process happened to hash there.

Not verified on a live Linux kernel from here. The parsers are pinned by captured output and the ratio test asserts the claim where it can run; CI on ubuntu-latest is what actually exercises the file reading.
