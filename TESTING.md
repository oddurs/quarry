# Testing and durability

Two ideas hold this up. Nothing in the core reaches the outside world directly,
and nothing in the core performs a side effect. Everything else follows from
that.

## The seams

Every external dependency is behind a trait in `src/source.rs`, so the whole
discovery pipeline can run with no machine underneath it:

| Trait | Real | Fixture |
|---|---|---|
| `SocketSource` | `lsof::Lsof` | `StaticSockets` (incl. a failing one) |
| `ProcessSource` | `procs::SysProcesses` | `StaticProcesses` |
| `CwdSource` | `lsof::LsofCwds` | `StaticCwds`, `NoCwds` |
| `Prober` | `probe::NetProber` | `probe::ScriptedProber` |

`Engine::live()` wires the real ones. `tests/pipeline.rs` wires fixtures to
captured `lsof` output and a repository layout built in a temp directory, so the
same assertions hold on every machine.

## No side effects in the core

`App` never opens a browser, never touches the clipboard, never signals a
process. It returns an `Action`, and `dispatch` in `src/main.rs` — the only code
that reaches outside — carries it out and reports back with a toast.

This is enforced, not just intended: `no_key_performs_a_side_effect_on_its_own`
drives every key with every modifier and fails if any of them asks to signal a
process without a confirmation first.

The rule exists because it was broken once. An early version wired `Enter`
straight to `open::that_detached`, and the randomised input test — which
presses `Enter` a few hundred times — opened a few hundred browser tabs.

## The suites

| Suite | What it protects |
|---|---|
| unit tests (in each module) | parsers, git resolution, the probe pool's lifecycle, the `exec` timeout |
| `tests/pipeline.rs` | sockets → processes → repos → the grouped list, from fixtures |
| `tests/rendering.rs` | golden screens, including colour |
| `tests/invariants.rs` | randomised states, inputs and terminal sizes |
| `tests/live_sockets.rs` | real loopback sockets: HTML, JSON, silence, garbage |
| `tests/terminal.rs` | the real binary under a real pty: hangup, signals, restore, live reload |
| `tests/performance.rs` | budgets, so a regression fails rather than being noticed a year later |

Identification has its own layer of defence, because the failures are quiet:
a wrong signature does not crash, it *confidently mislabels*. Three rules exist
because each was violated on a real machine, and each is now a test:

- a bare port names nothing — it announced "Dex" for whatever had bound 4470
- a process name must land on a word boundary — `serve` matched `redis-server`,
  `dex` matched `index.ts`
- a path is where a program lives, not what it is — `code` matched
  `/Users/someone/code/anything`

`quarry why <port>` found all three, one command each. A classifier nobody can
interrogate is one people argue with rather than fix.
| `tests/cli.rs` | the command line contract, against the real binary |

### Golden screens

`tests/snapshots/*.txt` are committed renderings. A change to the screen shows
up as a diff in review, which is the point.

```sh
cargo test --test rendering                  # check
QUARRY_UPDATE_SNAPSHOTS=1 cargo test         # accept a deliberate change
```

`standard_colours.txt` snapshots the foreground colour of every cell, so a
styling regression is caught as well as a layout one.

### Randomised testing

`tests/invariants.rs` generates states from an explicit seed — long names,
unicode, absurd ports, every health variant — drives random key sequences, and
renders at random sizes. It asserts three things: no panic, `check_invariants`
holds after every input, and nothing is ever drawn outside the terminal.

```sh
cargo test --test invariants                        # 80 seeds, the default
QUARRY_FUZZ_SEEDS=100000 cargo test --test invariants --release
```

CI runs 4000 seeds on every pull request. A failure prints the seed, and
replaying that seed reproduces the case exactly.

Bugs this has already caught: a collapsed group that stranded the cursor so it
could not be re-expanded, a branch name that pushed the service count off the
row, a service row that overflowed below about 70 columns, and the browser-tab
incident above.

Two properties are asserted against randomly generated *keymaps* as well:
no binding a config file can produce reaches a signal without a confirmation,
and quit is always reachable. Both found bugs. The second one is subtle — a
config that reassigns `q` and `ctrl-c` to other actions used to get a restored
quit binding appended behind the one that took `q`, so the help overlay
advertised a key that did nothing.

`App::check_invariants` is the contract:

- the selection is in range and never rests on an expanded group header
- every row points at a service or group that exists
- the group counts add up to the number of visible services

## Performance

`tests/performance.rs` is not a benchmark suite; it is a set of budgets. Each
one is set well above what the code costs today, so it never fails for noise,
and fails loudly if something becomes an order of magnitude slower. Run it with
`--nocapture` and the numbers are the useful part:

```sh
cargo test --release --test performance -- --nocapture
```

Two kinds of assertion, and the second is the better one:

- **Absolute budgets** scale by build profile, since a debug build is about ten
  times slower and CI runs both. One number in the source, from a release build.
- **Ratios** — the native socket source against `lsof`, a frame with 4000
  services against a frame with 30 — need no scaling, because a ratio holds in
  either profile. They also state the intent directly: not "this is fast" but
  "this is why we did it".

What the measurements changed, in the order they were found:

| | before | after | |
|---|---|---|---|
| a scan | 111 ms | 5.2 ms | the kernel directly instead of `lsof` |
| one probe result | 188 µs | 51 ns | index by pid; adjust one counter, not every group |
| a frame, 2000 services | 1.07 ms | 0.36 ms | build the visible rows, not all of them |
| filtering, one keystroke | 1.09 ms | 0.17 ms | lowercase the needle once, not per service |
| idle | 10 frames/s | 1 frame/s | redraw when something changed |

The first is the one that mattered: `lsof` was 98% of the cost of a scan, and
the profile said so before any code was written. Nothing here was guessed.

## Durability

**The one hard dependency is gone.** Listening sockets come from `libproc`
directly on macOS — the same interface `lsof` uses, without spawning it or
parsing its output back. `lsof` remains the fallback, and `--doctor` reports
which path is live and what each costs. The FFI struct layouts are checked three
ways: sizes and field offsets asserted against a C program compiled from the
SDK's own header, a short-write guard that refuses to read a buffer the kernel
filled differently than expected, and a test comparing the native source's
output against `lsof`'s on the machine running it.

**Nothing blocks forever.** Subprocesses go through `exec::run`, which drains
the pipes on a separate thread — a full pipe buffer is its own deadlock — and
kills the child on a deadline. Probes have connect and request timeouts and run
on a fixed pool, so one hung service delays only itself.

**Nothing grows without bound.** The scan is capped at `MAX_SERVERS` and says so
when it truncates. The probe queue is capped and drops rather than backs up. The
message channel is bounded, and the scanner drops an update rather than blocking
on a busy UI. Response bodies are read up to 96 KB.

**A failure degrades instead of blanking.** When a scan fails, the last good data
stays on screen, marked stale, with the reason in the status bar and the full
history behind `d`. The scanner backs off exponentially rather than retrying a
broken `lsof` every six seconds, and the UI reports it if the scanner thread
dies.

**The terminal always comes back, and the process always leaves.** `term::Guard`
restores on drop and on panic, via a hook that also logs the panic. Beyond that:

- `SIGTERM`, `SIGHUP` and `SIGQUIT` set a flag the event loop checks. `SIGPIPE`
  is ignored so a closed pipe cannot kill a render mid-frame.
- A flag is not enough on its own. When the terminal window closes, crossterm's
  `read` spins on the dead descriptor at 100% CPU and never returns, so the loop
  never gets to check anything. `term::input_closed` polls for `POLLHUP` *before*
  reading, which is the only point at which the loop can still act.
- Nor is that enough on its own. A terminal that stops draining its pty blocks
  the process inside `write` — including the write that restores the screen. So
  a watchdog thread takes the process down roughly 400ms after any termination
  signal, attempting the restore on a thread it is willing to abandon.

**The terminal is left as it was found.** Only the modes quarry actually uses are
requested — button reports and SGR encoding. Crossterm's `EnableMouseCapture`
also turns on `?1002` and `?1003`, drag and any-motion tracking, which make a
terminal emit a report for every movement of the mouse; quarry never reads them,
and leaving one enabled turns a shell into a wall of `zsh: command not found:
35;23;17M`. It happened. The reset is written as a single `write` — not
`execute!`, which abandons the rest of its sequence if any part fails — and it
disables every mouse mode, including the ones quarry never enables, because the
terminal may have been handed to us with one already on.

`tests/terminal.rs` holds all of this down under a real pty: closing the
terminal, `SIGTERM`, `SIGHUP`, a clean `q`, mouse reports arriving on stdin,
that motion tracking is never enabled, and — the general form — that *every*
mode turned on at startup is turned off at exit, checked on the wire rather than
in the code. Before those tests existed, closing the terminal left an orphan
spinning at 99% CPU, and quitting left the shell in motion-reporting mode.

`quarry --fix-terminal` is the escape hatch for a terminal any program left in a
bad state.

**Colour is data, and the terminal owns it.** Every colour is a *role* resolved
through a `Theme`; nothing in the drawing code names a colour. The default maps
every role to an ANSI slot so the terminal's own palette shows through, which is
asserted rather than assumed: `the_auto_theme_paints_nothing_absolute` fails on
a single RGB value in a rendered frame. `mono` must emit no colour at all, and
the health glyphs (`●`, `▲`, `✕`, `○`) carry the distinction that colour would,
so the display survives `NO_COLOR`, a colour-blind reader and a black-and-white
screenshot.

**Failures are visible.** `diag` keeps a bounded ring the `d` overlay shows, and
`QUARRY_LOG=/path/to/file` appends everything to disk. `quarry --doctor` checks
every dependency and exits non-zero if a fatal one is broken.

**Bug reports are reproducible.** `quarry --screenshot 120x40` renders one frame
as text with no terminal involved, so a report can carry the actual screen.

## Running it

```sh
./scripts/check          # fmt, clippy -D warnings, all tests, binary smoke test
./scripts/install-hooks  # run the above on every push
```

CI runs the same on Linux and macOS, plus an extended fuzz job and an MSRV
check.
