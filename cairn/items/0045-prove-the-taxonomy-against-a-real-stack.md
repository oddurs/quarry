---
id: 45
title: Prove the taxonomy against a real stack
type: chore
status: backlog
milestone: v0.4
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-08
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

- [ ] a Compose stack covering at least one service per kind
- [ ] a test asserting each is discovered, named, and healthy
- [ ] skipped cleanly with a clear message where no runtime is available
- [ ] run in CI on Linux
- [ ] reports the share of services it could not classify, and fails if that
      share grows
