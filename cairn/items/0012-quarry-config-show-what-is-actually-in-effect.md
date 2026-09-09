---
id: 12
title: quarry config — show what is actually in effect
type: feature
status: done
milestone: v0.2
depends_on:
- 11
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: cli
effort: s
---

## Problem

Once settings come from a file, an environment variable and a flag, "why is it
doing that" becomes a question the tool has to be able to answer. cairn solves
this with `cairn config`; quarry should not solve it differently.

## Proposal

- `quarry config` prints the resolved configuration as TOML, with the path it
  came from, and each value marked as default or overridden.
- `quarry config --write` writes a fully commented default file to the config
  path, refusing to clobber an existing one without `--force`.
- `quarry themes` lists every resolvable theme with its source: built-in, user
  file, or Ghostty.
- `--doctor` gains two lines: the config path in use, and the resolved theme.

## Acceptance criteria

- [ ] `quarry config` output is valid TOML that round-trips back through the
      loader
- [ ] `quarry config --write` refuses to overwrite without `--force`
- [ ] `quarry themes` marks the source of each theme
- [ ] `--doctor` reports the config path and the resolved theme name

## 2026-09-08

Also add `--theme <spec>` as a flag, which overrides the config for one run — the fastest way to try a theme before committing to it, and what a bug report should be asked to include.
