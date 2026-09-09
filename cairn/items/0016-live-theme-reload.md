---
id: 16
title: Live theme reload
type: feature
status: done
milestone: v0.2
depends_on:
- 8
created: 2026-09-08
updated: 2026-09-08
priority: p3
area: theme
effort: s
---

## Problem

Editing a theme means quitting quarry, editing, and starting it again. That is
a slow loop for the one task — choosing colours — that is pure trial and error.

## Proposal

Reload the config and theme on demand, and show the result immediately. A key
(`ctrl-r`, since `r` is rescan) re-reads both files and redraws. A parse failure
toasts the error and keeps the theme currently on screen, so a half-typed file
does not blank the display.

Watching the file for changes would be better still, but it costs a watcher
thread and a dependency; the key is most of the value for none of that. Revisit
only if the key proves annoying.

## Acceptance criteria

- [ ] a key reloads config and theme, redrawing at once
- [ ] a parse failure toasts and keeps the current theme
- [ ] the reloaded theme applies to overlays and the detail pane, not just the
      list — a partial repaint is worse than none
