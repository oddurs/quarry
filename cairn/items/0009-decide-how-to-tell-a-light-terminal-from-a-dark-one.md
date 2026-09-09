---
id: 9
title: Decide how to tell a light terminal from a dark one
type: spike
status: done
milestone: v0.2
depends_on:
- 7
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: theme
effort: s
---

## Question

Under the `auto` theme quarry does not know the terminal's background. Some role
choices depend on it: what "faint" means, whether a dim grey recedes or vanishes,
whether reverse-video selection reads at all. How should quarry find out?

## Why it has to be answered before the work

It decides whether `auto` is one mapping or two, and whether the theme loader
needs an async startup step. The theme files and the config schema both sit on
top of that answer, so guessing here is expensive to undo.

## Options

1. **Ask the terminal.** OSC 11 (`\e]11;?\a`) returns the background as
   `rgb:RRRR/GGGG/BBBB`. Widely supported — Ghostty, iTerm2, kitty, WezTerm,
   foot, recent xterm — but it needs a read with a timeout during startup, and a
   terminal that ignores it must not cost a visible pause.
2. **`COLORFGBG`.** Free and instant, set by rxvt and a few others. Absent in
   Ghostty, so on its own it answers almost nobody.
3. **Do not ask.** Assume dark; let `theme = "paper"` say otherwise. No startup
   cost, no compatibility surface, and wrong by default for the minority on a
   light terminal.
4. **Reverse video everywhere.** Sidesteps the question for selection, but not
   for `faint` and `muted`.

## What would settle it

Measure OSC 11 against the terminals actually in use here — Ghostty first — and
find what a non-responding terminal costs with a 100ms timeout. If it is
reliably fast, option 1 with 2 as a hint and 3 as the fallback. If it is flaky,
ship 3 and let the config carry it.

## Answer

<!-- Filled in when the spike closes. -->

## 2026-09-08

## Answer

**Ask, with `COLORFGBG` as a free hint and dark as the fallback** — option 1,
measured rather than assumed.

Implemented as `term::query_background`, called once from `Guard::new` in raw
mode before anything else reads stdin, with a 120ms budget. Measured under a pty
harness (`--theme auto`, `COLORFGBG` unset):

| terminal                     | cost to quarry |
|------------------------------|----------------|
| answers `rgb:0a0a/0f0f/1414` | 0 ms           |
| answers the BEL-terminated form | 0 ms        |
| ignores the query entirely   | 121 ms, once   |

A terminal that answers does so immediately; one that does not costs a single
120ms wait at startup, before the first frame, which is not perceptible. So
there is no reason to skip the question.

Three details the implementation depends on:

- It must run **in raw mode and before the event loop starts**, or the reply is
  either line-buffered or eaten by crossterm's reader. `query_background`
  returns `None` immediately if raw mode is not on, so calling it from the wrong
  place fails loudly rather than hanging.
- Both terminators are real: `ESC \` and `BEL`. Ghostty sends one, plenty of
  others send the other, and parsing only one would silently mean "no answer".
- `COLORFGBG` is checked first because it is free, but it is absent in Ghostty
  and most modern terminals, so on its own it would answer almost nobody.

`auto` therefore stays a single mapping parameterised by one boolean, and the
theme loader needs no async step — only this one synchronous question, asked
before the screen exists.
