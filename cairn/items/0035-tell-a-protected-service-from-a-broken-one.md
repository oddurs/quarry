---
id: 35
title: Tell a protected service from a broken one
type: feature
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
created: 2026-09-08
updated: 2026-09-15
priority: p1
area: probe
effort: s
---

## Problem

A `401` or `403` is drawn in the same yellow as a `404`, under a heading that
means "something is wrong". Nothing is wrong: the service is running and doing
exactly what it should. Anything behind auth — Grafana, Keycloak, a private API,
an admin panel — reads as a problem it is not.

The same is true in the other direction. A dev server that is still compiling
answers nothing for thirty seconds and reads as `no answer`, which is the same
thing quarry says about a service that has crashed.

## Proposal

Three states the health model does not currently have:

- **protected** — answered, and asked who you are. `401` and `403` are healthy.
- **starting** — the port is open and the service is not answering yet.
  Distinguishable from dead by being newly seen: a service whose socket appeared
  within the last minute and has not answered is starting, not broken.
- **degraded** — answering, but its own health endpoint says it is unhappy. A
  `200` on `/` and a `503` on `/healthz` is worth showing differently from both.

Each needs a glyph as well as a colour, since colour is not load-bearing here.

## Acceptance criteria

- [x] `401` and `403` read as protected, and do not count toward the unhealthy
      total in the title bar
- [x] a service first seen recently and not yet answering reads as starting
- [x] each state has its own glyph, so `mono` and a colour-blind reader see the
      distinction
- [x] snapshots per theme for the new states

## 2026-09-08

Partly done. **Protected** is in: 401 and 403 now read as `protected`, with their own glyph (◆) so the distinction survives `mono` and a colour-blind reader, their own colour role in every theme, and a sort rank alongside the working services rather than the failures. They never counted toward the unhealthy total — `is_trouble` was already 5xx-only — but they were drawn in the same yellow as a 404, which is the thing that teaches you to ignore the colour.

**Starting** and **degraded** are not done. Both need state quarry does not yet keep: when a socket was first seen, and the result of a health path separate from `/`.

## 2026-09-15

Starting is in. The state it needed — when a socket was first seen — arrived with the arrival highlight, so this became interpretation rather than new bookkeeping: the probe reports what it saw, and the app reinterprets a non-answer as starting when the service appeared within the last minute and something was expected to answer. That last clause is what keeps a newly started Redis at 'open', which is already its right and final answer.

Degraded is split out as 0046. It is not a health-model gap: quarry probes one path per service, so it cannot see a 200 on / and a 503 on /healthz at once. Two answers means two requests, and doubling the probe budget for a state most services are never in is the wrong trade to make without deciding how to bound it.

## 2026-09-15

Found while reviewing this branch, but in code this branch does not touch, so it is 0047 rather than a second change here.
