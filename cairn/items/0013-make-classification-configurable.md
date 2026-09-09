---
id: 13
title: Make classification configurable
type: feature
status: done
milestone: v0.2
depends_on:
- 11
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: config
effort: m
---

## Problem

`classify` holds a list of ports and process names compiled into the binary. It
is a good guess about common software and it is wrong about anybody's private
service on port 9174. Today the only fix is a pull request.

## Proposal

Config sections that extend — not replace — the built-in rules, so a user adds
what they run without losing what quarry knows:

    [ports]
    9174 = "queue"
    7000 = "api"

    [names]
    "my-daemon" = "api"
    "*-worker"  = "queue"      # simple glob, since argv is messy

    [health]
    3000 = "/healthz"          # probe this path rather than /
    "*"  = "/"

The health path matters more than it looks: a dev server that 404s on `/` shows
red in the list while being perfectly healthy, which trains you to ignore the
colour.

User rules win over built-in ones. `quarry config` shows the merged result so a
rule that never fires is visible.

## Acceptance criteria

- [ ] `[ports]`, `[names]` and `[health]` merge over the built-in tables
- [ ] a user rule overrides a built-in for the same port or name
- [ ] the health path is per-port with a `*` default
- [ ] `quarry config` shows merged rules, marking which came from the file
