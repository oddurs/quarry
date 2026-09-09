---
id: 29
title: Listen to what a service says when you connect
type: feature
status: done
milestone: v0.4
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: probe
effort: m
---

## Problem

quarry connects, learns that the connection succeeded, and reports `open`. Five
services on the machine this was written on show exactly that and nothing more.

But many protocols introduce themselves. Connecting to port 32222 here returns
`SSH-2.0-OrbStack\r\n` before a single byte is sent. quarry already opened that
connection and threw the answer away.

Protocols that volunteer a banner: SSH, SMTP, IMAP, POP3, FTP, NATS, MySQL
(its greeting carries the version), MongoDB in some configurations, Redis when
misconfigured, and a long tail of others.

## Proposal

After connecting, wait briefly for the server to speak first. If it does, match
the bytes against the signature table and the service is identified — including
its implementation and often its version — for the cost of a read on a
connection already open.

The cost is the wait for services that stay silent, which is most HTTP servers.
See 0033 for the sequencing; the short answer is one connection that peeks, then
speaks HTTP if nothing arrived.

## Acceptance criteria

- [ ] a banner read with its own short budget, distinct from the HTTP timeout
- [ ] banner patterns in the signature table, matched as bytes not as UTF-8 —
      MySQL's greeting is binary
- [ ] the banner outranks the port when they disagree
- [ ] the detail pane shows what the service said, verbatim and truncated
- [ ] version extracted where the banner offers one
- [ ] tested against a local server that sends each of: an SSH banner, an SMTP
      greeting, a MySQL greeting, and nothing at all
