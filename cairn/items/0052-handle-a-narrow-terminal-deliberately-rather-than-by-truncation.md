---
id: 52
title: Handle a narrow terminal deliberately rather than by truncation
type: feature
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
created: 2026-09-16
updated: 2026-09-16
priority: p1
area: chrome
effort: m
---

## Problem

quarry degraded as the terminal narrowed, but per row rather than per frame,
and the result was ragged.

`mail` is four characters and `metrics` is seven, so at a width where one fitted
and the other did not, one row kept its kind and the next lost it — and the
columns stopped lining up down the page, which is the only thing a column is
for.

Below about thirty columns it stopped being ragged and started being wrong: the
title bar drew its clock straight through `quarry`, and a group heading ran its
name into its count — `no project✕1  4`.

And on a wide terminal every spare column went to the list, where it became
whitespace between a service's name and its latency, while the pane holding
paths and command lines stayed at its minimum and wrapped them.

## Proposal

- Decide the columns once for the whole list, from the width and the widest
  label and kind on screen. A column is present for every row or for none.
- Drop them in the order they can be spared: the kind first, since the colour
  already carries it, then the latency, then the name takes what is left.
- Give the detail pane away before the list gives up a column. On a narrow
  terminal the list is the tool.
- Above what a row needs, spare width goes to the detail pane rather than the
  list.

## Acceptance criteria

- [x] every row shows the same columns, at every width
- [x] nothing overflows, at any width a terminal can be
- [x] the title bar never paints over itself
- [x] the detail pane gives way before the list loses a column
- [x] a wide terminal gives its extra room to the detail pane

## 2026-09-16

The raggedness had one cause: the badge decision was per row, and the badge width varies by kind. Hoisting it into a Columns computed once per frame fixed it and made the rest of the degradation expressible as an order — kind, then latency, then the name takes what is left.

Two collisions below thirty columns that only showed up because the tests swept every width from twenty to two hundred: the title bar draws two widgets over one line and they were each fitted to the area rather than to each other, so the clock painted through the identity; and a group heading's budget did not account for the gutter or leave a gap, so the name ran into the count.

The wide case was the opposite mistake. Spare width all went to the list, where it became whitespace between a name and a latency, while the pane holding paths and command lines stayed at its minimum and wrapped them. The list now takes what a row needs and the detail pane takes the rest.

MIN_LIST is not a round number: it is what one complete row needs plus borders. Buying the detail pane by taking a column off every row in the list is the wrong way round.
