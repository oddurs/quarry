---
id: 32
title: See UDP listeners
type: feature
status: done
milestone: v0.4
depends_on:
- 28
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: discovery
effort: m
---

## Problem

quarry is blind to UDP. That is DNS resolvers, QUIC and HTTP/3 origins, StatsD,
syslog, WireGuard, mDNS, game servers, SIP, and every emulator that speaks it.

A developer running CoreDNS or dnsmasq for local resolution, or testing an
HTTP/3 endpoint, sees nothing at all.

## Proposal

The native source already enumerates every socket; a bound UDP socket is the
same walk with a different `soi_kind`. UDP has no listening state, so "bound
with no peer" is the closest honest equivalent and is what `lsof -iUDP` shows.

Two things are genuinely different and should be shown as such rather than
papered over:

- **There is no connection to test.** A UDP service is `bound`, not `open`; the
  only way to know more is to speak its protocol, which is what makes the DNS
  handshake in 0034 worth having.
- **UDP is noisy.** mDNS, DHCP and Bonjour are on every machine and belong under
  the same system-noise rule that hides `rapportd` today.

## Acceptance criteria

- [ ] UDP listeners discovered natively, marked as a different transport
- [ ] health reads `bound` — never `open`, which would imply a test that did not
      happen
- [ ] a service listening on both TCP and UDP on one port is one row
- [ ] system UDP noise hidden by default
- [ ] `--plain` names the transport, so a script can tell them apart
