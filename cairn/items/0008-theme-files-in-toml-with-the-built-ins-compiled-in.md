---
id: 8
title: Theme files in TOML, with the built-ins compiled in
type: feature
status: done
milestone: v0.2
depends_on:
- 6
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: theme
effort: m
---

## Problem

`auto` inherits the terminal, which is right by default and wrong when you want
quarry to look like something specific — a light theme for a screenshot, a
high-contrast one for a projector, or simply a palette you prefer.

## Proposal

A theme is a TOML file of roles. Ship four, compiled in with `include_str!` so
they are parsed by the same code path users' files go through and cannot drift
from the format:

- `auto` — the terminal's own palette (see 0007)
- `gotham` — the palette this project is actually developed under
- `night` — the current Tokyo Night colours, kept so nothing is lost
- `paper` — a light theme, because a dark-only tool is unusable in sunlight

Resolution order for `theme = "<spec>"`:

1. a built-in name
2. `~/.config/quarry/themes/<spec>.toml`
3. a path, if it looks like one

Every colour in a file is optional and unspecified roles inherit from the
default, so a three-line file that only changes the accent is valid.

## Acceptance criteria

- [ ] `Theme::from_toml`, with hex, `#rgb`, ANSI index and named colours accepted
- [ ] four built-ins, each parsed at startup like a user's file
- [ ] a partial theme file fills its gaps from the default rather than failing
- [ ] a broken theme file names the file, the line and the bad value, and falls
      back to the default rather than refusing to start
- [ ] a snapshot per built-in theme
