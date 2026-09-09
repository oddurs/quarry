---
id: 23
title: Stop redrawing ten times a second while nothing happens
type: feature
status: done
milestone: v0.3
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: runtime
---

## Problem

The event loop redrew on every 100ms tick whether or not anything had changed.
quarry is meant to sit open, and a tool that sits open should not cost anything
to sit open.

## Proposal

Track what actually moves: an arriving message, a handled keystroke, the scan
spinner, the "updated N ago" clock ticking over, a toast appearing or expiring.
Redraw when one of those happened — and unconditionally once a second anyway, so
a missed flag costs a second of staleness rather than a wrong screen.

## Result

Idle: ten frames a second → **one**, measured at 0.3% of one core over twelve
seconds including two scans.
