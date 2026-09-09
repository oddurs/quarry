---
id: 25
title: Budget the hot paths so a regression fails CI
type: chore
status: done
milestone: v0.3
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: testing
---

## Problem

Every number in this milestone is one somebody could put back by accident. A
performance win that nothing defends is a performance win with a shelf life.

## Proposal

`tests/performance.rs`: budgets rather than benchmarks. Each is set well above
what the code costs today, so it never fails for noise, and fails loudly on an
order-of-magnitude regression. Printed with `--nocapture`, the numbers are the
useful part.

Two kinds, and the second is better:

- **Absolute budgets**, scaled by build profile — a debug build is about ten
  times slower and CI runs both, so one number lives in the source and the debug
  run scales it.
- **Ratios** — native against `lsof`, a frame with 4000 services against one
  with 30 — which need no scaling, hold in either profile, and state the intent
  directly: not "this is fast" but "this is why we did it".

CI runs the suite in release as well as debug.

## Also

`examples/breakdown` prints the profile of a scan. It is kept because it, not a
guess, decided every change in this milestone.
