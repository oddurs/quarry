---
id: 31
title: Decide the probe budget once banners and handshakes are in play
type: spike
status: done
milestone: v0.4
depends_on:
- 29
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: probe
effort: s
---

## Question

quarry currently probes HTTP first, because that costs one connection instead of
two on the common path. Adding a banner read and protocol handshakes changes
the arithmetic: what sequence identifies the most services for the least
latency, and what is the budget per service?

## Why it has to be answered before the work

Every probe ticket in this milestone sits on the answer. Get it wrong and either
a scan takes seconds on a machine with fifty services, or half of them stay
unidentified because we did not wait long enough to hear them speak.

## Options

1. **One connection: connect, peek, then speak.** Wait briefly for a banner; if
   nothing arrives, send the HTTP request down the same socket. One connection,
   both answers. Costs the peek window on every HTTP service, and means owning
   the HTTP exchange rather than handing the socket to `ureq`.
2. **Peek only when the signature suggests it.** Ports and process names already
   hint; only wait for a banner where something says one is likely. Fast, and
   circular — a service we cannot classify is exactly the one we most need to
   hear from.
3. **Two phases.** HTTP first as today; anything that comes back `open` gets a
   second, slower pass with the banner read and any handshake the signature
   names. Nothing gets slower, unidentified services get identified a moment
   later, and the pool does twice the work for them.
4. **Peek with zero wait.** Read whatever is already buffered and never block.
   Free, and catches only servers that wrote before we looked — which, on a
   loopback connection, may be most of them.

## What would settle it

Measure. How long after `connect` does a banner actually arrive on loopback —
for OrbStack's SSH on 32222, for a local Postfix, for MySQL? If it is reliably
under a few milliseconds, option 4 or 1 with a very short window is nearly free
and the question is settled. Then measure a whole scan of fifty services under
each option and pick on the numbers, as `examples/breakdown` did for `lsof`.

## Answer

<!-- Filled in when the spike closes. -->

## 2026-09-08

## Answer

**Option 3, two phases** — measured, not reasoned.

Banner arrival on loopback, against a server that greets immediately:

| wait after connect | banner caught | median total |
|---|---|---|
| 0 ms | **0 / 30** | — |
| 1 ms | 30 / 30 | 0.11 ms |
| 5 ms | 30 / 30 | 0.12 ms |

Three things follow, and together they settle it:

- **A zero-wait peek catches nothing.** Not "usually nothing" — 0 out of 30, from
  a server that writes its greeting the instant it accepts. The bytes are not
  there yet when `connect` returns. Option 4 is dead.
- **The wait is free when a banner is coming.** `recv` returns the moment data
  arrives, so a 1 ms window and a 200 ms window both cost 0.11 ms against a
  talkative server. The window is a *ceiling*, not a cost.
- **The wait is paid in full by silent servers.** A 50 ms window costs 51 ms
  against port 3000; a 200 ms window costs 201 ms. Every HTTP server on the
  machine is silent, so option 1 — peek before speaking, on one connection —
  taxes the common case to serve the rare one.

Real numbers from this machine: OrbStack's SSH on 32222 needed **~50 ms**, not
5 ms, because it is a real sshd behind a VM boundary. So a window has to be tens
of milliseconds, which is exactly the amount option 1 would spend on everything.

### The shape

Phase one is unchanged: HTTP first, one connection, no waiting. Anything that
answers is identified and done, and nothing gets slower than it is today.

Phase two runs only for what phase one could not identify — the services that
currently report `open` and nothing else, five of them here. Those get a fresh
connection, a **250 ms** banner window (generous, because the set is small and
a VM-hosted service can be slow), and any handshake the signature names.

Services whose kind already says they do not speak HTTP skip phase one entirely
and go straight to the banner and handshake.

The set that pays the cost is precisely the set with no information, which is
the right place to spend it.
