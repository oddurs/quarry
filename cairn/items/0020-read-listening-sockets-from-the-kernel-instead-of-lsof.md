---
id: 20
title: Read listening sockets from the kernel instead of lsof
type: feature
status: done
milestone: v0.3
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: discovery
---

## Problem

`lsof` was 98% of the cost of a scan — 76ms of 78 — because it walks every file
descriptor of every process on the machine and formats the answer as text for us
to parse back. quarry then does it again six seconds later. It was also the one
binary quarry could not run without.

## Proposal

Ask the kernel the same questions `lsof` asks, through `libproc`:
`proc_listpids`, then `proc_pidinfo(PROC_PIDLISTFDS)` per process, then
`proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` per socket descriptor, keeping the ones
in `TSI_S_LISTEN`.

`lsof` stays as the fallback for any platform or situation the native path
cannot cover, and `--doctor` reports which one is live and what each costs.

## Result

| | |
|---|---|
| native (libproc) | **1.7 ms** |
| lsof | 77 ms |

A whole scan went from 111ms cold to 5ms.

## Getting the FFI right

A wrong struct layout reads plausible garbage rather than failing, so three
things guard it:

- Sizes and field offsets are asserted against a C program compiled from the
  SDK's own `<sys/proc_info.h>`.
- A short-write guard refuses to read a buffer the kernel filled differently
  than expected.
- A test compares the native source's output against `lsof`'s on the machine
  running the tests, and fails on systematic disagreement.

The bug that cost the most time: `PROC_PIDFDSOCKETINFO` is a flavour for
`proc_pidfdinfo`, not `proc_pidinfo`. Both take a flavour argument, and 3 means
`proc_bsdinfo` to the latter — so the first version returned 136 bytes of the
wrong structure and read as an empty machine.
