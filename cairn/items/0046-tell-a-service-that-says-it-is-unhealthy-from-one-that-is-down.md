---
id: 46
title: Tell a service that says it is unhealthy from one that is down
type: feature
status: backlog
milestone: v0.4
depends_on:
- 35
created: 2026-09-15
updated: 2026-09-15
priority: p2
area: probe
effort: m
---

## Problem

Split out of 0035, which delivered **protected** and **starting**. The third
state in that item's proposal was **degraded**: a service answering `200` on
`/` while its own health endpoint says `503`.

quarry cannot see that today, and not because of the health model. It probes
one path per service — the signature's health path where there is one, `/`
otherwise — so it sees one of those two answers and never both. A `503` on the
health path is already drawn as a `503`, which is honest as far as it goes.

The distinction worth drawing is between *the front door is broken* and *the
service has diagnosed itself as unwell*. Those call for different reactions and
currently look identical.

## Proposal

Two answers means two requests, and that is the whole cost of this item: the
probe budget was sized in 0031 for one request per service plus a banner read.
Doubling it for every HTTP service to catch a state most of them are never in
is the wrong trade.

Cheaper options, in order of preference:

- Ask the health path only, and treat a `2xx` on it as healthy however the
  root behaves. Free, and arguably already what a health path means.
- Ask the second path only when the first answered `2xx` *and* a signature
  names a health path distinct from `/`. Bounded to services that declared one.
- Ask it on a slower cadence than the main probe — every fourth scan, say.

## Acceptance criteria

- [ ] a service answering on `/` and failing its own health path reads as
      degraded, with its own glyph
- [ ] the extra request is bounded: it does not run for every service on
      every scan
- [ ] the probe budget in `tests/performance.rs` still passes

## 2026-09-15

A second source of 'degraded' arrived with 0038: a gRPC server answering NOT_SERVING on grpc.health.v1.Health/Check. The answer is read and shown in the detail pane already; what is missing is the same thing an unhealthy /healthz is missing, which is a health state that means 'answering, and saying it is unwell'. Both should land together.
