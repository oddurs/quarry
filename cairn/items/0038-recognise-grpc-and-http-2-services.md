---
id: 38
title: Recognise gRPC and HTTP/2 services
type: feature
status: backlog
milestone: v0.4
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-08
priority: p2
area: probe
effort: m
---

## Problem

A gRPC server speaks HTTP/2 with prior knowledge and no TLS. `ureq` sends an
HTTP/1.1 request, gets nothing it understands, and quarry reports `open`. Every
gRPC backend a developer runs is currently invisible as such.

The same is true of any h2c-only service, and of HTTP/3, which is not TCP at all
and needs 0032 first.

## Proposal

Two steps, cheap and in order:

- **Detect h2c.** Send the HTTP/2 connection preface. A server that speaks it
  replies with a SETTINGS frame; nothing else does. That alone distinguishes
  "gRPC or h2c server" from "a socket that ignored us".
- **Ask it how it is.** gRPC has a standard health service,
  `grpc.health.v1.Health/Check`. A `SERVING` response is a real health answer
  from a service that currently has none.

gRPC-Web and Connect ride on HTTP/1.1 and already answer today; they need only a
signature entry to be named correctly.

## Acceptance criteria

- [ ] h2c detected by preface and SETTINGS, without a full HTTP/2 client
- [ ] the gRPC health check attempted where a signature says gRPC, with
      `SERVING` mapped to healthy and `NOT_SERVING` to degraded
- [ ] a server with no health service reads as running, not as broken — most do
      not implement it
- [ ] tested against a scripted server that returns a SETTINGS frame
