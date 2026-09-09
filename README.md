# quarry

See every server running on this machine — and whose project it came from.

**[oddurs.github.io/quarry](https://oddurs.github.io/quarry)** · MIT

`quarry` scans every listening TCP socket, maps it back to the process that owns
it, walks up from that process's working directory to find the git repository it
belongs to, and probes each one for health. The result is a TUI grouped by
project, with clickable URLs.

A project is the repository a service is running in. If it is not in a
repository, the directory it is running in is the next best answer, and quarry
uses that — a repo name is shown in magenta, a plain folder in teal.

```
 quarry  21 listening · 8 repos · 1 unhealthy                              updated 2s ago
─────────────────────────────────────────────────────────────────────────────────────────
╭ Services ──────────────────────────────╮╭ Detail ──────────────────────────────────────╮
│ ▾ orchard  chore/monorepo-structure 4 ││ ● ledger-api                                  │
│  ●  3000 next-server     web  200  5ms ││   node index.ts · web · pid 49019            │
│  ●  3001 next-server     web  200  3ms ││                                              │
│ ▾ typeset  main  oddurs/atlas  ✕1  2 ││ ADDRESS                                      │
│  ●  4320 node serve      web  404  2ms ││   http://localhost:4470  ↗                   │
│  ●  4330 node astro.mjs  web  500  3ms ││ ...                                          │
╰────────────────────────────────────────╯╰──────────────────────────────────────────────╯
 ↑↓ move  ↵ open  y copy  / filter  a all  K stop  r refresh  ? help
```

## What it knows

quarry ships with signatures for about 560 services — databases, brokers,
proxies, dev servers, debuggers, notebooks, emulators, game servers — and
identifies them by scoring evidence rather than guessing from a port:

```
quarry why 3000       # what matched, what it scored, what it lost to
quarry signatures     # everything quarry knows
```

A port alone names nothing; it suggests a category. What names a service is a
banner it volunteered, a title on its own page, or the program you launched. If
quarry does not know yours, four lines in
`~/.config/quarry/signatures.toml` teach it — see [SIGNATURES.md](SIGNATURES.md).

## Theming

quarry uses your terminal's colours. There is nothing to configure — if your
terminal is set to Gotham, quarry is Gotham, and it follows when you change it.

When you want something else:

```sh
quarry --theme gotham        # for one run
quarry themes                # everything quarry can find
quarry themes gruv           # filtered — Ghostty ships hundreds
```

Or permanently, in `~/.config/quarry/config.toml`:

```toml
theme = "ghostty:gotham"     # your terminal's own theme file
```

Built in: `auto` (the default), `mono` (no colour at all), `gotham`, `night`,
`paper`. Your own files go in `~/.config/quarry/themes/`. `NO_COLOR`,
`--no-color` and `TERM=dumb` all select `mono`, where the status glyphs carry
what the colours would have.

See [THEMES.md](THEMES.md) for the role table and how to write one.

## Configuration

Everything else lives in `~/.config/quarry/config.toml`, and every key is
optional.

```sh
quarry config --write        # a commented file with every default
quarry config                # what is actually in effect, and where it came from
```

```toml
theme         = "auto"
show_all      = false   # include system services on startup
refresh_secs  = 6
connect_ms    = 400
request_ms    = 1800
probe_workers = 12

# Ports quarry does not already know about. These merge over the built-in
# table, so you add what you run without losing what quarry knows.
[ports]
9174 = "queue"

# Matched against the command and its arguments. `*` matches anything.
[names]
"*-worker" = "queue"

# A dev server that 404s on / reads as unhealthy while being perfectly fine,
# which trains you to ignore the colour. Give it the path that means healthy.
[health]
3000 = "/healthz"

# Keys, bound to action names. Run `quarry --help`, or press `?`, for the
# defaults — the help overlay is generated from your bindings, not from ours.
[keys]
"x" = "stop"
```

`ctrl-r` re-reads both files and repaints, so choosing colours does not mean
restarting. A file that will not parse is reported and the running theme is
kept.

## Install

```sh
brew install oddurs/tap/quarry                       # macOS and Linux
cargo install --git https://github.com/oddurs/quarry  # from source
curl -fsSL https://oddurs.github.io/quarry/install.sh | sh
```

Prebuilt binaries for macOS and Linux, on Apple silicon and x86, are attached to
[every release](https://github.com/oddurs/quarry/releases) with checksums.

## Use

```sh
quarry                        # the TUI
quarry --all                  # include system services
quarry --plain                # one line per service, for scripts
quarry --theme gotham         # use a theme for this run
quarry config                 # show the configuration in effect
quarry themes                 # list every theme quarry can find
quarry --doctor               # check everything quarry depends on
quarry --screenshot 120x40    # render one frame as text, no terminal needed
quarry --fix-terminal         # undo a terminal left in mouse-reporting mode
```

`QUARRY_LOG=/tmp/quarry.log` appends diagnostics to a file.

### Keys

| key | |
|---|---|
| `↑` `↓` / `j` `k` | move between services |
| `g` / `G` | first / last |
| `space` `←` `→` | collapse or expand a repo group |
| `enter` / `o` | open the URL in your browser |
| click | select a row, or open the URL in the detail pane |
| `y` | copy the URL to the clipboard |
| `/` | filter by repo, port, process or kind |
| `a` | include system services |
| `esc` | back out — clear the filter, close an overlay |
| `r` | rescan now |
| `K` / `X` | SIGTERM / SIGKILL the process, with a confirm |
| `d` | diagnostics — what failed, and why |
| `ctrl-r` | reload the config and theme |
| `m` | toggle mouse capture — off restores native text selection |
| `?` | help |
| `q` | quit |

## How it works

- **Every transport**, not just TCP: unix domain sockets and bound UDP ports
  too. On a typical machine there are several hundred unix sockets and a couple
  of dozen ports, and a great deal of local software — the Docker daemon,
  PostgreSQL, PHP-FPM, anything using socket activation — is reachable only
  through one. They live behind `--all` with the rest of the background.
- **Sockets** come from the kernel directly, through `libproc` — the same
  interface `lsof` uses, without spawning it or parsing its output back. That is
  about forty times faster (a scan is 5 ms rather than 110 ms) and removes the
  one binary quarry depended on. `lsof` stays as the fallback; `quarry --doctor`
  says which path is live.
- **Process detail** comes from `sysinfo`; a second batched `lsof` fills in any
  working directory it could not read.
- **Projects** come from the working directory: the repository it sits in, or
  failing that the directory's own name. The executable's location is only
  consulted when there is no working directory at all, and never for a package
  manager's prefix — otherwise every Homebrew-installed daemon would claim to
  belong to Homebrew's own repository.
- **Colour** is a set of *roles*, not a palette. The default maps every role
  onto the terminal's own ANSI slots, so quarry inherits whatever theme is
  already on screen. See [THEMES.md](THEMES.md).
- **Repos** are found by walking up from the working directory for a `.git`.
  Branch and remote are read out of the git directory directly rather than by
  shelling out, so a full scan stays under a millisecond. A linked worktree is
  grouped under the repository it belongs to, not under its own directory name.
- **Health** is a `GET /` for anything that might answer one, on a pool of 12
  threads. Whatever that cannot identify gets a second pass: quarry connects,
  waits a moment to see whether the service introduces itself — SSH, SMTP, NATS
  and MySQL all do — and asks directly where it knows the protocol. Only the
  services nothing else could identify pay for that.
- Everything rescans every six seconds, and probe results stream in as they land
  so a hung service never blocks the rest of the list.

## Cost

quarry is meant to sit open, so it is built not to be noticed:

- A scan is about **5 ms**, every six seconds.
- Idle, it redraws **once a second**, not ten times — measured at 0.3% of one
  core with two scans in the window.
- A frame costs the same whether the machine is running 30 services or 4000;
  only the rows on screen are built.

`tests/performance.rs` holds all of that to a budget, so a regression fails CI
rather than being noticed a year later.

## Notes

- Classification is port-first with the process name as a tie-breaker, which is
  what you want when half the machine is called `node`.
- Services bound to `0.0.0.0` are flagged in the detail pane — that is a port
  your network can reach.
- System and desktop-app listeners (Dropbox, Adobe, `rapportd`, mDNS, …) are
  hidden unless you pass `--all` or press `a`.

## Durability

- Every subprocess has a deadline and is killed if it overruns; every probe has
  connect and request timeouts.
- Scan size, probe queue, message queue and response bodies are all bounded.
- A failed scan keeps the last good data on screen, marked stale, and backs off
  rather than hammering a broken `lsof`.
- The terminal is restored on quit, on panic, on `SIGTERM`/`SIGHUP`/`SIGQUIT`,
  and through an exit hook that covers every other way the process can leave.
  Only clicks and scrolling are requested — never motion tracking, which floods
  a terminal with a report per mouse movement.
- If some *other* program leaves your shell in mouse-reporting mode,
  `quarry --fix-terminal` clears it.
- `quarry --doctor` tells you which dependency is broken; `--screenshot` makes a
  bug report reproducible.

## Development

```sh
./scripts/check          # fmt, clippy, tests, smoke test — what CI runs
./scripts/install-hooks  # run the above on every push
```

Nothing in the core touches the machine directly or performs a side effect —
external dependencies sit behind traits, and `App` returns actions for the shell
to carry out. That is what makes the suites below possible. See
[TESTING.md](TESTING.md).

```sh
cargo test                                   # everything
QUARRY_UPDATE_SNAPSHOTS=1 cargo test         # accept a deliberate screen change
QUARRY_FUZZ_SEEDS=100000 cargo test --release --test invariants
```
