---
id: 38
title: Recognise gRPC and HTTP/2 services
type: feature
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-15
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

- [x] h2c detected by preface and SETTINGS, without a full HTTP/2 client
- [x] the gRPC health check attempted where a signature says gRPC, with
      `SERVING` and `NOT_SERVING` both read and reported. Mapping
      `NOT_SERVING` onto a *degraded health state* is 0046, which owns that
      state — it did not exist when this was written
- [x] a server with no health service reads as running, not as broken — most do
      not implement it
- [x] tested against a scripted server that returns a SETTINGS frame

## 2026-09-15

h2c detection needed no HTTP/2 client: the preface plus an empty SETTINGS frame, answered by a SETTINGS frame, and nothing that is not an HTTP/2 server answers at all. It slots into the existing handshake dispatch as another named probe.

The health check needed a little more, but far less than a client. HPACK is used only in its literal-without-indexing form, which never touches the dynamic table, so nothing has to be remembered between frames — and the reply is read from the DATA frame, where the status is a two-byte protobuf, rather than from the HPACK-encoded trailers.

Writing it turned up a bug that a scripted-server test would never have caught, because I would have written the server to match the encoder: content-type is HPACK static index 31, and the literal-without-indexing form has a four-bit prefix. Written as one octet, 31 reads as a different header type and the server misparses everything after it. The integer encoding is now asserted against RFC 7541's own worked examples rather than against a round trip through this file.
