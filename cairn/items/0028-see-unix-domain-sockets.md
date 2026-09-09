---
id: 28
title: See unix domain sockets
type: feature
status: done
milestone: v0.4
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: discovery
effort: m
---

## Problem

quarry looks at TCP and nothing else. On the machine this was written on, that
misses **107 unix domain sockets from `com.docker`, 36 more from Docker, 24 from
node, 13 from wrangler, 11 from OrbStack** — and those are just the listeners.

This is not an edge case. A great many local services listen on a unix socket
either as well as, or instead of, a port: the Docker daemon, PostgreSQL's
`/tmp/.s.PGSQL.5432`, MySQL, PHP-FPM, gunicorn `--bind unix:`, containerd,
Redis, systemd and launchd socket activation. A tool that claims to show what is
running on your machine cannot be blind to all of it.

## Proposal

The native source already walks every file descriptor of every process; a unix
socket is `SOCKINFO_UN` where TCP is `SOCKINFO_TCP`, and the path is right
there. Extend `Listener` to carry an address that is either a socket address or
a filesystem path.

A unix socket has no port, so several things need an answer rather than a
default:

- **Sort and group** by path, shown as `~/.orbstack/run/docker.sock` rather than
  a number.
- **Probing** it means connecting to the path — which HTTP happily runs over
  (that is exactly how the Docker API works), so the same prober applies with a
  different transport.
- **The URL** is not a URL. Copying gives the path.

Only listening sockets, not every connected pair, or the list becomes noise.

## Acceptance criteria

- [ ] unix listeners discovered natively alongside TCP
- [ ] the list and detail pane render a path where a port would go, without the
      layout assuming a number
- [ ] HTTP probing works over a unix socket, tested against the Docker API
- [ ] a filter matches on the path
- [ ] hidden behind the same system-noise rule, since most of these are not the
      user's
