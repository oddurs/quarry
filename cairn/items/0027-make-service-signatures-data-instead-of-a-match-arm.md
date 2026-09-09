---
id: 27
title: Make service signatures data instead of a match arm
type: feature
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: discovery
effort: l
---

## Problem

`classify` is a Rust `match` over ports and substrings. Every new kind of server
means editing it, and everything a signature could carry — what to call it, what
path means healthy, what URI scheme to copy, what it says on connect — has no
home, so each of those lives in a different function keyed by the same
guesswork.

Supporting the gamut of servers by extending that match is a losing game: there
are hundreds of them, they change, and the person who knows about yours is you.

## Proposal

A **signature** is data. One table, compiled in as TOML like the themes, and
extensible by the user in `~/.config/quarry/signatures.toml`:

```toml
[[signature]]
name    = "PostgreSQL"
kind    = "db"
ports   = [5432, 5433]
process = ["postgres", "postmaster"]
banner  = ""                       # postgres does not volunteer one
probe   = "postgres"               # a named handshake, when one exists
uri     = "postgres://localhost:{port}/"

[[signature]]
name    = "Grafana"
kind    = "observability"
ports   = [3000]
http    = { server = "*", title = "Grafana*", path = "/api/health" }
health  = "/api/health"
```

Matching is scored rather than first-hit, because the evidence disagrees: port
3000 says "web", a `Grafana` title says Grafana, and the title should win. Each
matched field contributes, and the highest score names the service. A banner or
an HTTP fingerprint outranks a port, because a port is a convention and a banner
is the service speaking.

This is the keystone for the rest of this milestone: with it, every protocol
below is a data addition and a probe function, not a change to the classifier.

## Acceptance criteria

- [ ] signatures parse from TOML, built-ins compiled in, user file merged over
- [ ] scored matching, with evidence ranked: banner > HTTP fingerprint > process
      name > port
- [ ] a signature carries display name, kind, health path and URI template
- [ ] the existing built-in table is expressed as signatures with no change in
      what today's tests see
- [ ] `quarry signatures` lists them and says which matched what, so a wrong
      guess can be diagnosed rather than argued about
- [ ] user signatures win over built-in ones at equal score

## 2026-09-08

Done, and it earned itself twice over. 564 signatures across 22 kinds, scored rather than first-match. Three rules came out of real misidentifications on this machine, each now a test: a bare port names nothing (it announced 'Dex' for whatever bound 4470); a process name must land on a word boundary (`serve` matched `redis-server`, `dex` matched `index.ts`); and a path is where a program lives, not what it is (`code` matched `/Users/someone/code/...`). `quarry why` found all three in one command each.
