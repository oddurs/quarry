---
id: 15
title: Respect NO_COLOR and a non-colour terminal
type: feature
status: done
milestone: v0.2
depends_on:
- 7
created: 2026-09-08
updated: 2026-09-08
priority: p2
area: theme
effort: s
---

## Problem

quarry emits colour unconditionally. `NO_COLOR` is a convention it should
honour, and `TERM=dumb` or a pipe is a situation it should notice.

## Proposal

- `NO_COLOR` set to anything non-empty, or `--no-color`, selects a `mono` theme
  where every role is `Reset` and emphasis is carried by bold, dim and reverse.
- `TERM=dumb` or an absent `TERM` does the same.
- `--color=always` overrides all of it, for piping into something that renders.

Because emphasis has to survive without colour, this is also the check that the
layout does not depend on colour to be readable — which is the same thing a
colour-blind user needs.

## Acceptance criteria

- [ ] `NO_COLOR`, `--no-color`, `TERM=dumb` all select the mono theme
- [ ] `--color=always` wins over `NO_COLOR`
- [ ] a snapshot of the mono theme, checked for it emitting no colour at all
- [ ] health status stays distinguishable without colour — the glyph carries it
