---
id: 45
title: Prove the taxonomy against a real stack
type: chore
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-16
priority: p1
area: testing
effort: m
---

## Problem

Every claim in this milestone is checkable, and none of it is checked by a test
that resembles the situation it is for. The unit tests use fixtures; the live
tests use sockets this repository opens. Neither tells us whether quarry
identifies a real Postgres, a real Redis or a real Grafana.

Without that, "first class support" is a list of signatures nobody has run
against the software they describe.

## Proposal

A Compose file in `tests/fixtures/stack/` that brings up a representative
spread — Postgres, Redis, MongoDB, RabbitMQ, MinIO, Grafana, Prometheus,
Mailpit, a plain nginx, a gRPC server — and an integration test that scans,
and asserts each is found, named and reported healthy.

Ignored by default, since it needs a container runtime: `cargo test --ignored
--test stack`. Run in CI on Linux, where a runtime is already there.

This is also the honest way to measure the headline number. "What fraction of
services are classified as `other`" means nothing against fixtures we wrote; it
means something against a stack somebody would actually run.

## Acceptance criteria

- [x] a Compose stack covering at least one service per kind
- [x] a test asserting each is discovered, named, and healthy
- [x] skipped cleanly with a clear message where no runtime is available
- [x] run in CI on Linux
- [ ] reports the share of services it could not classify, and fails if that
      share grows

## 2026-09-15

Blocked on 0048, which this test found: eleven containers published by one runtime process collapse into one row. The test cannot pass until that is fixed, and weakening it to pass would remove the only reason it exists.

## 2026-09-16

It found two bugs before it ever passed, which is the whole argument for it.

0048: a container runtime publishes every port from one process, so eleven containers arrived on one row named after whichever the first listener resolved to. Every fixture and every live test had at most one container, and with one container the old code is correct.

0049: with that fixed, all eleven were kind container rather than db, cache or queue, because identify ran against the host process — which is the runtime.

A third thing the test got wrong about itself: waiting for a port to be listening is not waiting for the service. A runtime binds the forwarder as soon as the container is created, well before the software inside is up, and compose up --wait does not help because most of these images declare no healthcheck. It now scans and probes in a loop until everything answers.

And the two tests became one. They shared a stack, and two Drop guards racing to tear it down meant whichever finished first pulled it out from under the other.
