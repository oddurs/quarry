# Themes

quarry uses your terminal's colours by default. If you have already chosen a
theme for your terminal, there is nothing to configure — quarry wears it, and
follows when you change it.

Everything below is for when you want something else.

## Choosing a theme

```sh
quarry --theme gotham      # for one run
quarry themes              # everything quarry can find
quarry themes gruv         # filtered, because Ghostty ships hundreds
```

Permanently, in `~/.config/quarry/config.toml`:

```toml
theme = "gotham"
```

A theme name resolves in this order:

1. `auto` — the terminal's own palette. The default.
2. `mono` — no colour at all; emphasis by bold, dim and reverse.
3. A built-in: `gotham`, `night`, `paper`.
4. `~/.config/quarry/themes/<name>.toml`.
5. A path, if it is one.
6. `ghostty:<name>`, or any bare name matching a Ghostty theme.

An unresolvable name falls back to `auto` and says so in the diagnostics pane
(`d`), rather than refusing to start.

## How `auto` works, and what it costs

`auto` maps every role onto an ANSI slot (0–15) or `reset`. The terminal
substitutes its configured colours, which is what makes quarry match Gotham,
Solarized, or whatever else is already on screen — and keep matching when you
switch.

Sixteen slots is not many, so two things are given up:

- Roles that want a shade *between* two slots do not get one. `surface` and
  `background` are the same colour, so panes do not sit on a raised ground.
- Selection is reverse video rather than a background tint, because a tint
  needs to know what it is tinting.

If you want those, use a theme file. `ghostty:<name>` is usually the least work:
it has the full palette your terminal is already using.

## Reading the terminal's own theme

```toml
theme = "ghostty:gotham"
```

quarry reads Ghostty theme files directly from:

- `~/.config/ghostty/themes/`
- `/Applications/Ghostty.app/Contents/Resources/ghostty/themes/`
- `/usr/share/ghostty/themes/`

Bright palette slots are preferred over normal ones where a role has to carry
over a dark background. That is not an accident: the official Gotham port fills
its bright slots with background shades, and honouring that is what makes it
look right rather than washed out.

## Writing a theme

A theme file names **roles** — what a colour is for — not colours. That is what
lets a palette be swapped without touching any drawing code, and it is what you
need to know to write one.

Every key is optional. Anything you leave out keeps its `auto` value, so a
one-line file is valid:

```toml
accent = "#edb54b"
```

### Structure

| role | what it is |
|---|---|
| `background` | the page behind everything |
| `surface` | panes, a shade above the background |
| `overlay` | popups, which have to read as floating above the page |
| `border` | pane outlines at rest |
| `border_focus` | the outline of whatever has focus |
| `selection` | the ground under the selected row |
| `selection_reverse` | `true` uses reverse video instead of `selection` |

### Text

| role | what it is |
|---|---|
| `text` | body text |
| `muted` | secondary text that still has to be read |
| `faint` | separators, rules, things you look past |
| `heading` | section headings |

### Emphasis

| role | what it is |
|---|---|
| `accent` | the one note used sparingly — keys, the app's name |
| `secondary` | a supporting emphasis |

### Health

These are the colours a glance depends on, so they want real separation.

| role | what it is |
|---|---|
| `ok` | 2xx |
| `redirect` | 3xx |
| `client_error` | 4xx |
| `server_error` | 5xx |
| `open` | a socket that accepts but does not speak HTTP |
| `closed` | not responding |
| `unknown` | not probed yet |

Health also carries a glyph — `●`, `▲`, `✕`, `○` — so the distinction survives
`mono`, a colour-blind reader, and a screenshot in black and white.

### Projects

| role | what it is |
|---|---|
| `repo` | a service in a git repository |
| `folder` | a service in a plain directory |
| `generic` | the catch-all groups |

A repository is a firmer claim than a directory that merely has a name, so keep
these distinguishable.

### Service kinds

The badge column, which is how you scan the list. Under `[kinds]`:

```toml
[kinds]
web = "#33859d"
api = "#195466"
db  = "#888ca6"
```

The full set: `web api db cache search queue proxy mail container ai tool
system other`.

### Colour values

Any of these work anywhere a colour is expected:

```
"#0a0f14"      "#f80"        "0a0f14"
"rgb:0a/0f/14" "rgb:0a0a/0f0f/1414"
"4"            "ansi:12"      — an ANSI slot; follows the terminal
"bright-cyan"  "magenta"      — a colour name
"reset"                       — whatever the terminal already uses
```

An ANSI slot or `reset` is how a theme says "follow the terminal for this one
role" while pinning the rest.

A value that cannot be parsed leaves its role alone and reports the problem; it
does not invalidate the file.

## Checking your work

```sh
quarry --theme ./mine.toml --screenshot 120x40
```

Renders one frame as text with no terminal involved. Useful for a bug report,
and for seeing the layout without the colours getting in the way.
