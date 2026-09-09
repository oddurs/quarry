---
id: 21
title: Stop rebuilding every group on every probe result
type: feature
status: done
milestone: v0.3
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: chrome
---

## Problem

Each probe result called `rebuild_groups_only`, which walked every row and, for
each, searched every group by comparing an allocated `group_key()` string. On a
machine with 2000 services that was 188µs per result and 2000 results per scan —
0.4 seconds of work every six seconds, to update some counters.

## Proposal

Two indexes, both built once per scan alongside the rows:

- `by_pid: HashMap<u32, Vec<usize>>` so a result finds its services directly
  rather than by walking the list.
- `group_of: Vec<usize>` so a service knows its group without deriving the key.

A health change then adjusts one counter, and only when the service crossed the
line between healthy and not.

## Result

188µs → **51ns** per result, and `tests/performance.rs` holds it there.
