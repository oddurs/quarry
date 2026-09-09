---
id: 5
key: v0.2
title: Themes and configuration
type: milestone
status: backlog
created: 2026-09-08
updated: 2026-09-08
priority: p2
---

quarry currently hardcodes a Tokyo Night palette in RGB, which overrides
whatever the terminal is already set to. Someone running Gotham sees quarry in
somebody else's colours.

This milestone makes the default *inherit the terminal*, adds real theme files
for when it should not, and puts the rest of the tool's behaviour — refresh
rate, timeouts, classification, keys — in a config file.

Done when: quarry running under Gotham looks like Gotham, without configuration.
