---
id: 49
title: A containerised service is classified as a container, not as what it is
type: bug
status: backlog
milestone: v0.4
depends_on:
- 48
created: 2026-09-15
updated: 2026-09-15
priority: p1
area: discovery
effort: s
---

## Problem

Also found by 0045. With 0048 fixed, an eleven-service stack produces eleven
rows — and every one of them is kind `container`:

    55432  container  postgres
    56379  container  redis
    58080  container  nginx

A PostgreSQL in a container is a database. `identify()` runs against the host
process, and for a published port the host process is the runtime —
`com.docker.backend` — which classifies as `container` and is not wrong about
itself. It is simply not the service.

For a milestone called "first-class support for every kind of server", the
kinds are the taxonomy, and containerised services are outside it entirely.
Most people run their databases in containers.

## Proposal

The daemon already told us the image: `postgres:16-alpine`, `redis:7`,
`nginx:1`. That is a better name than anything the host process offers, and the
signature table already matches `postgres`, `redis` and `nginx` by process
name — so feeding the image in as evidence should classify most of the stack
without a single new signature.

The container's own name is still what is *shown*: `stack-postgres-1` is what
the user called it. This is about the kind, and about the signature behind it.

## Acceptance criteria

- [ ] a container's image is used as evidence when identifying it
- [ ] the stack in 0045 classifies by kind rather than all as `container`
- [ ] the displayed name is still the container's, not the image's
- [ ] a container whose image matches nothing is still `container`
