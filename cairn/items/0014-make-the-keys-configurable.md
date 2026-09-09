---
id: 14
title: Make the keys configurable
type: feature
status: done
milestone: v0.2
depends_on:
- 11
created: 2026-09-08
updated: 2026-09-08
priority: p2
area: config
effort: m
---

## Problem

The keymap is a `match` in `App::handle_key`. It is a reasonable default and it
is somebody else's default: `K` for stop and `X` for force kill are not obvious,
and anyone with muscle memory from another tool has to relearn them.

## Proposal

Name every action, then let the config bind keys to names:

    [keys]
    "ctrl-r" = "refresh"
    "x"      = "kill"
    "s"      = "open"

An action is a stable string, so a binding survives a refactor of the match arm
behind it. Unbound actions keep their defaults; a binding to an unknown action
warns rather than failing.

Two things must stay non-negotiable and should be tested as such: quit is always
reachable, and nothing that signals a process can be bound to a single key
without the confirmation step. A config file must not be able to arm a keystroke
that kills a process.

The `?` help overlay must render the *bound* keys, not the defaults — a help
screen that lies is worse than none.

## Acceptance criteria

- [ ] named actions, bound by string in `[keys]`
- [ ] an unknown action or unparseable key warns and is skipped
- [ ] the help overlay reflects the active bindings
- [ ] a test asserts no binding can reach a signal without a confirmation
- [ ] a test asserts quit is always bound to something
