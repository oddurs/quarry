---
id: 6
title: Replace the hardcoded palette with colour roles
type: feature
status: done
milestone: v0.2
assignee: Oddur Sigurdsson
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: theme
effort: m
---

## Problem

`src/theme.rs` is fifteen `Color::Rgb` constants named after colours — `GREEN`,
`MAGENTA`, `FAINT` — and the drawing code reaches for them directly. Two things
follow from that. Nothing can be re-themed without editing the drawing code, and
the names describe pigment rather than purpose, so there is no way to say what a
colour is *for* and let a theme decide what it looks like.

## Proposal

Introduce a `Theme` struct of **roles**, and pass it to the drawing code. A role
says what a thing is; a theme says what colour that is.

Structure: `background`, `surface`, `overlay`, `border`, `border_focus`,
`selection`.
Text: `text`, `muted`, `faint`, `heading`.
Emphasis: `accent`, `secondary`.
Health: `ok`, `redirect`, `client_error`, `server_error`, `open`, `closed`,
`unknown`.
Projects: `repo`, `folder`, `generic` — a repository is a firmer claim than a
directory and the two must stay distinguishable.
Kinds: one colour per `Kind`, since the badge column is how you scan the list.

This item is a refactor with no visible change: the default theme reproduces
today's palette exactly, and the existing snapshots must not move. That is the
point — it makes the change reviewable.

## Acceptance criteria

- [ ] `Theme` struct with a role per use, no colour named after a colour
- [ ] `ui.rs` takes the theme from `App`; no `theme::` constants remain in it
- [ ] the built-in default reproduces the current palette byte for byte
- [ ] every existing snapshot passes unchanged
- [ ] a test asserts every role is read somewhere, so a dead role is caught
