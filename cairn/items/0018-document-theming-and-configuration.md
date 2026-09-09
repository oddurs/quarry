---
id: 18
title: Document theming and configuration
type: docs
status: done
milestone: v0.2
depends_on:
- 12
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: docs
effort: s
---

## Problem

None of this is discoverable from the binary alone. Someone with a terminal
theme they like needs to be told, in one line, that quarry already wears it.

## Proposal

- README: a short "Theming" section leading with the default — quarry uses your
  terminal's colours, and there is nothing to configure. Then the escape hatches:
  a built-in theme, your Ghostty theme, your own file.
- `THEMES.md`: the role table — what each role is *for*, which is what someone
  writing a theme needs and what a list of colour names cannot tell them. A
  worked example building a theme from scratch.
- README: a commented `config.toml` showing every key with its default.
- A screenshot under Gotham, since that is the case this milestone exists for.

## Acceptance criteria

- [ ] README leads with "it already matches your terminal"
- [ ] every role documented by purpose, not by colour
- [ ] every config key documented with its default
- [ ] the example config in the docs is the one `quarry config --write` emits,
      so the two cannot drift
