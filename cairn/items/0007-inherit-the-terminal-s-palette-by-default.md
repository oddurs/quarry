---
id: 7
title: Inherit the terminal's palette by default
type: feature
status: done
milestone: v0.2
depends_on:
- 6
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: theme
effort: m
---

## Problem

quarry paints in 24-bit RGB taken from Tokyo Night. A terminal set to Gotham,
Solarized or anything else is simply overruled — the tool looks foreign in its
own window, and it does not follow when the terminal's theme changes.

## Proposal

Add an `auto` theme, and make it the default. It maps every role to an **ANSI
indexed colour (0–15)** or `Color::Reset`, so the terminal substitutes its own
palette. Gotham in Ghostty then renders quarry in Gotham with no configuration,
and re-theming the terminal re-themes quarry.

The mapping has to be chosen carefully, because 16 slots is not many:

- `background`, `text` → `Reset`, so the terminal's own ground and foreground
  show through and a transparent background stays transparent
- `faint`, `muted` → 8 (bright black) and 7
- health → 2 / 6 / 3 / 1 for ok / redirect / client error / server error
- `accent` → 4, `repo` → 5, `folder` → 6
- `selection` → reverse video rather than a background colour, which is the only
  way to be legible against a palette we cannot see

Roles that want a shade between two ANSI slots — a subtle selection ground, a
surface a step above the background — do not get one under `auto`. That is the
trade, and it is the right one: correct in every terminal beats ideal in one.

## Acceptance criteria

- [ ] `auto` is the default theme
- [ ] a test asserts `auto` emits only `Indexed`, `Reset` and named ANSI colours
      — one `Rgb` value in it defeats the whole feature
- [ ] transparent terminal backgrounds stay transparent
- [ ] snapshot of the `auto` theme's colour map, alongside the existing ones
