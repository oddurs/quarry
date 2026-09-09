---
id: 42
title: Show what the TLS certificate says
type: feature
status: backlog
milestone: v0.4
created: 2026-09-08
updated: 2026-09-08
priority: p2
area: probe
effort: s
---

## Problem

quarry disables certificate verification so that a local HTTPS service with a
self-signed certificate can be probed at all — which is the right call, and
means it currently knows something about every TLS service and reports none of
it.

Local TLS goes wrong in specific, recognisable ways: a `mkcert` certificate that
expired, one issued for a hostname that is not the one you are using, a service
that is HTTPS-only while something in front of it assumes HTTP.

## Proposal

Report what the handshake already produced: subject and SAN names, issuer,
expiry, and whether it would have verified. A certificate that has expired, or
expires within a week, is worth marking — that is a class of local failure that
is otherwise diagnosed by reading a browser error.

## Acceptance criteria

- [ ] subject, issuer and expiry in the detail pane for TLS services
- [ ] an expired or nearly-expired certificate marked in the list
- [ ] a name mismatch reported, since that is the common `mkcert` failure
- [ ] verification stays off — this reports, it does not enforce
