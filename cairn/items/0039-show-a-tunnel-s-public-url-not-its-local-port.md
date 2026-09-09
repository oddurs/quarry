---
id: 39
title: Show a tunnel's public URL, not its local port
type: feature
status: backlog
milestone: v0.4
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-08
priority: p2
area: attribution
effort: s
---

## Problem

ngrok and cloudflared exist to give a local service a public address. quarry
shows the local port they listen on, which is the one piece of information the
user already had.

## Proposal

Both publish their state locally. ngrok's agent API on `127.0.0.1:4040` returns
`/api/tunnels`, with the public URL and the local address each one forwards to.
cloudflared exposes metrics and its own config.

With that, a tunnel stops being a row of its own and becomes an annotation on
the service behind it: port 3000 shown as `acme-web` *and* reachable at
`https://something.ngrok.app`.

That also makes the most important thing about a tunnel visible: **it is
exposing a local service to the internet right now**, which is worth seeing at a
glance and worth being able to find in a hurry.

## Acceptance criteria

- [ ] ngrok tunnels read from the agent API and matched to the local service
- [ ] the public URL shown in the detail pane and copyable, alongside the local
      one
- [ ] a service currently exposed is marked in the list
- [ ] no agent running is not a failure
