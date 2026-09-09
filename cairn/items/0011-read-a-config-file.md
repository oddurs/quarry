---
id: 11
title: Read a config file
type: feature
status: done
milestone: v0.2
assignee: Oddur Sigurdsson
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: config
effort: m
---

## Problem

Everything quarry does is currently a constant compiled into it: a six second
refresh, a 400ms connect timeout, twelve probe workers, a fixed list of ports
that mean "database". None of it can be changed without a rebuild, and the
defaults are guesses about somebody else's machine.

## Proposal

`config.toml`, resolved in this order, first hit wins:

1. `$QUARRY_CONFIG`
2. `~/.config/quarry/config.toml` (`$XDG_CONFIG_HOME` if set)
3. built-in defaults

Deliberately no per-directory config: quarry looks at the whole machine, so a
config that changes depending on where you launched it would be a trap.

Every key optional, `#[serde(default)]` throughout, so a file naming one setting
is valid. Start with the settings that exist as constants today:

    theme          = "auto"
    show_all       = false      # include system services on startup
    refresh_secs   = 6
    connect_ms     = 400
    request_ms     = 1800
    probe_workers  = 12

A malformed config names the file and the offending line, then continues on
defaults. A tool that refuses to start because of a typo in an optional file is
worse than one that tells you and carries on — and it can tell you, because
there is a diagnostics pane to tell you in.

## Acceptance criteria

- [ ] `Config::load` with the resolution order above
- [ ] unknown keys warn into `diag` and are ignored, so a config written for a
      newer quarry still works
- [ ] a malformed file is reported and does not prevent startup
- [ ] `--config <path>` overrides the search entirely
- [ ] tests cover: missing file, empty file, partial file, malformed file,
      unknown key
