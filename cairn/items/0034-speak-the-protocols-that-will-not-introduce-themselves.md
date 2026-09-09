---
id: 34
title: Speak the protocols that will not introduce themselves
type: feature
status: backlog
milestone: v0.4
depends_on:
- 27
- 31
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: probe
effort: l
---

## Problem

A database is the most common thing a developer runs and the thing quarry knows
least about. Postgres, Redis, MongoDB and memcached all say `open` and nothing
more — no version, no confirmation it is actually the thing the port suggests,
no way to tell a healthy server from a port something else grabbed.

## Proposal

A handful of one-shot handshakes, named by the signature table so adding one is
data plus a function rather than a change to the classifier. Each sends a few
bytes and reads a reply; none require authentication, and none write anything.

| protocol | ask | learn |
|---|---|---|
| Redis | `PING` | `+PONG`, and `INFO server` gives the version |
| Postgres | `SSLRequest`, then a startup packet | speaks the protocol; TLS or not |
| MySQL | connect | the greeting carries the version (see 0029) |
| MongoDB | `hello` over the wire protocol | version, replica set role |
| memcached | `version\r\n` | version |
| AMQP | protocol header | it is RabbitMQ, and which version |
| MQTT | `CONNECT` | the broker answers `CONNACK` |
| DNS | an `A` query for `localhost` | it is a resolver, not a coincidence |

The point is not the version string. It is that a **port is a convention and a
handshake is proof**: something else sitting on 5432 stops being reported as
"PostgreSQL, healthy".

## Acceptance criteria

- [ ] handshakes named from the signature table, dispatched by name
- [ ] every one read-only, unauthenticated, and bounded by the probe budget
- [ ] a handshake that fails downgrades to `open`, not to `closed` — the socket
      is there, we just could not confirm what it is
- [ ] a mismatch is reported: "5432, expected PostgreSQL, did not answer like one"
- [ ] version shown in the detail pane where the protocol offers one
- [ ] tested against a scripted server per protocol, not against a live database

## 2026-09-08

Partly done. The mechanism is in place — `src/handshake.rs`, dispatched by a signature's `probe` field, with Redis, memcached, PostgreSQL's SSLRequest and MongoDB's `hello` implemented, plus a test asserting every `probe` named in the shipped table exists. AMQP, Kafka, MQTT and DNS remain. MySQL needed no handshake: its greeting arrives unprompted and 0029 covers it.

## 2026-09-08

Partly done. The mechanism is in — `src/handshake.rs`, dispatched by a signature's `probe` field, with Redis (`PING`), memcached (`version`), PostgreSQL (`SSLRequest`, eight bytes and provably not a login) and MongoDB (`hello`) implemented, each read-only and unauthenticated, and a test asserting every `probe` named in the shipped table exists. MySQL needed no handshake: its greeting arrives unprompted and 0029 covers it. AMQP, Kafka, MQTT and DNS remain.
