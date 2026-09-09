---
id: 37
title: Read listening sockets from /proc on Linux
type: feature
status: backlog
milestone: v0.4
depends_on:
- 28
created: 2026-09-08
updated: 2026-09-08
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

- [ ] a native Linux source, `lsof` still the fallback
- [ ] the inode join is done once for all sockets, not once per socket
- [ ] parses against captured `/proc/net/*` fixtures, so the tests run anywhere
- [ ] `--doctor` reports which source is live, as it does on macOS
- [ ] measured against `lsof` on Linux, with the ratio asserted as on macOS
