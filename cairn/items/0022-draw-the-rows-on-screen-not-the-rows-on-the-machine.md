---
id: 22
title: Draw the rows on screen, not the rows on the machine
type: feature
status: done
milestone: v0.3
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: chrome
---

## Problem

`draw_list` built a `ListItem` for every row and handed the lot to the widget so
it could display forty of them. The cost of a frame scaled with the size of the
machine rather than the size of the window.

## Proposal

Compute the visible window from the scroll offset and the pane height, build
items for that slice only, and manage the offset directly — keeping the cursor
in view with the least movement, so the list does not jump when it is already
visible.

## Result

A frame with 2000 services: 1.07ms → **0.36ms**, and now within a whisker of a
frame with 30. `a_frame_costs_the_same_on_a_busy_machine` asserts the ratio,
which holds in debug and release alike.
