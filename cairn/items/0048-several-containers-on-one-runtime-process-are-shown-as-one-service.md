---
id: 48
title: Several containers on one runtime process are shown as one service
type: bug
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
created: 2026-09-15
updated: 2026-09-15
priority: p0
area: discovery
effort: m
---

## Problem

Found by 0045, the first test to put quarry in front of a stack somebody would
actually run.

A container runtime publishes every port from one process. On this machine
`com.docker.backend`, pid 81937, holds all eleven published ports of an
eleven-service Compose stack. `Engine::assemble` builds one `Server` per pid, so
all eleven listeners land on one row — named after whichever container the
*first* listener happened to resolve to, with the other ten reported as `+10`.

    51211  container  quarry  memcached  open · 0.1ms

That is the only row. PostgreSQL, Redis, MongoDB, RabbitMQ, MinIO, Grafana,
Prometheus, Mailpit and nginx are all on screen as "memcached".

0030 attributed a published port to its container and this defeats it: the
attribution is right per port, and then ten of the eleven ports are thrown away
by the grouping that happens first.

A machine with one container published shows the right thing, which is why this
survived — every fixture and every live test had at most one.

## Proposal

The container is the service; the forwarder is not. Where one process's
listeners resolve to different containers, emit one `Server` per container
rather than one per pid.

Ports from that process that resolve to *no* container stay together as they do
now — that is the runtime's own listener, and it is one service.

## Acceptance criteria

- [x] one process publishing several containers produces one row per container
- [x] each row carries only its own container's ports
- [x] a process with no containers behind it is unchanged
- [x] a test with several containers on one pid, which is the case every
      existing fixture misses

## 2026-09-15

Found by 0045, which is the first test to put quarry in front of a stack somebody would run. Every fixture and every live test had at most one container, and with one container the old code is correct — which is exactly why it survived.

The engine had no seam for the container list: scan() calls Containers::query() unconditionally, so a test could not supply several. with_containers pins it, which is what made the regression test possible at all. Everything else in the pipeline was already drivable from fixtures.

A second problem is visible in the same output and is not fixed here: every containerised service comes out kind Container rather than db, cache or queue, because identify() runs against the host process — which is the runtime. Filed separately.
