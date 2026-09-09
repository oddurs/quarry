---
id: 10
title: Wear the terminal's own theme by reading Ghostty's theme files
type: feature
status: done
milestone: v0.2
depends_on:
- 8
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: theme
effort: m
---

## Problem

`auto` gets the terminal's sixteen palette slots and nothing else — no background
shade, no selection colour, no room between the slots. Ghostty already has the
full palette on disk, in a file whose name the user has already chosen. Asking
them to transcribe it into a quarry theme is asking them to maintain the same
palette twice.

## Proposal

Read Ghostty theme files directly, as almanac already does. Same format:
`key = value` lines plus `palette = N=#rrggbb`, no sections.

Look in, in order:

- `~/.config/ghostty/themes/`
- `/Applications/Ghostty.app/Contents/Resources/ghostty/themes/`
- `/usr/share/ghostty/themes/`

So `theme = "ghostty:gotham"` — or a bare name matching nothing built in —
resolves to the terminal's own file.

Reuse almanac's `from_ghostty` rather than reinventing it, including its
bright-slot-first choice: the official Gotham port fills the bright slots with
background shades, and honouring that is what makes it look right.

## Acceptance criteria

- [ ] `theme = "ghostty:<name>"` and a bare name both resolve
- [ ] a Ghostty file with only `palette` lines and no `background` still works
- [ ] tested against a checked-in copy of the Gotham Ghostty theme
