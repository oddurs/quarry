---
id: 17
title: Keep the tests honest once colours are data
type: chore
status: done
milestone: v0.2
depends_on:
- 6
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: testing
effort: m
---

## Problem

The snapshot suite pins colour with `standard_colours.txt`, and rendering was
just made a pure function of state so those snapshots stop drifting. A theme
system reintroduces exactly the hazard that was closed: what a screen looks like
would start depending on the machine's config file, its `NO_COLOR`, and whatever
Ghostty themes happen to be installed.

## Proposal

Make the theme an explicit input to every rendering test, never an ambient one.

- `App` carries its `Theme`; tests set it, the way they now set `App::now`.
- The snapshot suite pins one theme and asserts against it.
- A colour snapshot per built-in theme, so a change to any palette is a
  reviewable diff.
- The randomised suite renders under every built-in theme, since a role that is
  `Reset` in one and a real colour in another can expose a contrast bug that
  only appears in one of them.
- The CLI tests run with `QUARRY_CONFIG` pointed at a fixture, so a developer's
  own config can never change a test result.

## Acceptance criteria

- [ ] no test reads the ambient config, environment or theme directories
- [ ] a colour snapshot per built-in theme
- [ ] the fuzz harness cycles themes
- [ ] a test asserts `auto` contains no RGB values
- [ ] a test asserts every theme defines, or inherits, every role
