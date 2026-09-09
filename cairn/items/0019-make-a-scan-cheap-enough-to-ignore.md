---
id: 19
key: v0.3
title: Make a scan cheap enough to ignore
type: milestone
status: backlog
created: 2026-09-08
updated: 2026-09-08
priority: p2
---

quarry sits open. A scan every six seconds that costs 111ms is a tool you
notice, and `lsof` was 98% of it.

Everything in this milestone was decided by measurement, not by guessing:
`examples/breakdown` printed the profile before any of it was written.

Done when: a scan is a handful of milliseconds, a frame costs the same on a busy
machine as on an idle one, and a regression in either fails CI.
