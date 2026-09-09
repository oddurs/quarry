---
id: 35
title: Tell a protected service from a broken one
type: feature
status: backlog
milestone: v0.4
created: 2026-09-08
updated: 2026-09-08
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

- [ ] `401` and `403` read as protected, and do not count toward the unhealthy
      total in the title bar
- [ ] a service first seen recently and not yet answering reads as starting
- [ ] each state has its own glyph, so `mono` and a colour-blind reader see the
      distinction
- [ ] snapshots per theme for the new states

## 2026-09-08

Partly done. **Protected** is in: 401 and 403 now read as `protected`, with their own glyph (◆) so the distinction survives `mono` and a colour-blind reader, their own colour role in every theme, and a sort rank alongside the working services rather than the failures. They never counted toward the unhealthy total — `is_trouble` was already 5xx-only — but they were drawn in the same yellow as a 404, which is the thing that teaches you to ignore the colour.

**Starting** and **degraded** are not done. Both need state quarry does not yet keep: when a socket was first seen, and the result of a health path separate from `/`.
