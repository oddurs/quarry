---
id: 39
title: Show a tunnel's public URL, not its local port
type: feature
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-15
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

- [x] ngrok tunnels read from the agent API and matched to the local service
- [x] the public URL shown in the detail pane and copyable, alongside the local
      one
- [x] a service currently exposed is marked in the list
- [x] no agent running is not a failure

## 2026-09-15

ngrok only. cloudflared's local state is not one endpoint — it exposes Prometheus metrics on a port you choose, and the tunnel's hostname lives in its config file or in Cloudflare's API rather than in the agent. There is no equivalent of /api/tunnels to read, so it is a separate piece of work rather than a second line in the same function; the Exposure type carries the agent name so adding one does not change what is already there.

Two judgements worth knowing about. ngrok reports one tunnel per protocol, so a port forwarded over both http and https appears twice — https wins, since that is the one worth pasting. And copy gives the public URL where there is one while open still goes to the local address: copying is for sharing, opening is for looking at it yourself, and the short way round is better for the second.
