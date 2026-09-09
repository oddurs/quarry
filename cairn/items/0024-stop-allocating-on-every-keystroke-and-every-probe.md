---
id: 24
title: Stop allocating on every keystroke and every probe
type: chore
status: done
milestone: v0.3
created: 2026-09-08
updated: 2026-09-08
priority: p2
area: chrome
---

## Problem

Three allocation hot spots, all in code that runs constantly:

- `Server::matches` lowercased the command and the whole command line for every
  service on every keystroke, and the filter lowercased its needle again each
  time.
- `rebuild` called `group_key()` from inside a sort comparator, so it allocated
  O(n log n) strings to sort n services.
- Every HTTP probe built a fresh `ureq::Agent` — a TLS configuration and a
  connection pool constructed and thrown away per request, per service, every
  six seconds.
- The diagnostics ring dropped its oldest entry with `Vec::remove(0)`, shifting
  every remaining element on every event recorded.

## Proposal

Lowercase the needle once and match case-insensitively without allocating
(ASCII fast path, correct fallback); render a port's digits into a stack buffer
rather than a `String`; compute group keys once per rebuild and sort on the
result; build one agent per prober; use a `VecDeque` for the ring.

## Result

Filtering 2000 services on one keystroke: 1.09ms → **0.17ms**.
