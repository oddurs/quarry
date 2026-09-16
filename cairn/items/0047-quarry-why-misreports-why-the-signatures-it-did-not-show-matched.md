---
id: 47
title: quarry why misreports why the signatures it did not show matched
type: bug
status: backlog
created: 2026-09-15
updated: 2026-09-15
priority: p2
area: cli
effort: s
---

## Problem

`quarry why <port>` shows the five highest-scoring signatures and then prints

    …and {rest} more matching the port alone

unconditionally. `ranked` is every verdict at or above `MIN_SCORE`, sorted by
score descending — the tail is not necessarily port-only, and the shown rows
know this: each is individually labelled `(port alone — not enough to name it)`
or not, from `names_the_service()`. The summary line contradicts the logic
three lines above it.

Reproducible from the shipped table. Ten signatures list `firebase` in their
`process` array and eight list `firebase-tools`; seven list `beam.smp`. A
`firebase emulators:start` process therefore produces ten verdicts scoring
`W_PROCESS` or better, five are shown, and the other five are reported as
having matched the port alone when they matched the process name.

`quarry why` exists to explain a verdict honestly. A false sentence in its
output is worse here than anywhere else in the tool.

## Proposal

Count and label the remainder from the same predicate the rows use:

    let (named, port_only) = rest.partition(|v| v.names_the_service());

and say whichever is non-zero, rather than asserting one of them.

## Acceptance criteria

- [ ] the summary line's claim is derived, not assumed
- [ ] a test with more than five process-name matches on one port asserts the
      line does not say "port alone"
