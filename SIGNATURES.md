# Signatures

quarry knows about 560-odd services. When it meets one it does not know, telling
it takes four lines.

## How a service is identified

Identification is **scored**, not first-match, because the evidence disagrees.
Port 3000 says "some web thing"; a `<title>` saying `Grafana` says Grafana. The
title should win, and it does.

| evidence | weight | why |
|---|---|---|
| banner | 100 | the service introducing itself, unprompted |
| HTTP title | 70 | the service's own page, naming itself |
| HTTP `Server` | 50 | usually the service, sometimes a proxy in front |
| process name | 40 | what you actually launched |
| argument | 15 | a subcommand of something else — weak on its own |
| port | 20 | a convention, and a crowded one |

Scores **add**. A signature matching a port *and* a process name beats one
matching either alone, which is how two things on port 3000 are told apart.

Three rules matter more than the numbers:

- **A port alone names nothing.** It carries a *kind* — a guess about a category
  — but not a name. Several hundred signatures claim a port; letting a bare port
  match put a name on screen is how quarry once announced "Dex" for whatever had
  bound 4470.
- **A process name must land on a word boundary.** Plain substring matching made
  `serve` match `redis-server` and `dex` match `index.ts`.
- **A path is where a program lives, not what it is.** Matching runs against the
  names in a command line, not the directories on the way to them — otherwise
  `code` matches `/Users/you/code/anything`.

## Seeing why

```sh
quarry why 3000
```

```
3000 — pid 50445
  process   next-server (v16.3.4)
  verdict   Next.js (web)

  → Next.js                        60  process "next-server", port 3000
    serve                          20  port 3000            (port only — not enough to name it)
```

The whole ranking, with what each match rested on. A classifier nobody can
interrogate is one people argue with rather than fix.

```sh
quarry signatures            # everything quarry knows
quarry signatures postgres   # filtered
```

## Writing one

`~/.config/quarry/signatures.toml`. Merged over the shipped table; an entry with
the same name replaces the built-in one, and ties go to yours.

```toml
[[signature]]
name    = "Billing API"
kind    = "api"
ports   = [9174]
process = ["billing-api"]
health  = "/healthz"
```

### Fields

| field | |
|---|---|
| `name` | what to show. Required. |
| `kind` | one of the kinds below. Required. |
| `ports` | conventional ports. Omit rather than guess. |
| `process` | matched against argv. `*` is a wildcard; anything else needs a word boundary. |
| `banner_text` | an ASCII prefix the server sends **unprompted** on connect. |
| `banner` | the same in bytes: `hex:0a` for a binary greeting like MySQL's. |
| `http_server` | glob against the `Server` header. |
| `http_title` | glob against the HTML `<title>`. |
| `health` | the path that means healthy. `/` when unset. |
| `uri` | what you would paste into a client. `{port}` and `{path}` are filled in. |
| `probe` | a named handshake: `redis`, `postgres`, `memcached`, `mongodb`, `http`. |
| `note` | one line, shown in the detail pane. For something genuinely surprising. |

### Kinds

```
web api db cache vector search queue storage metrics proxy tunnel auth
registry container emulator workflow realtime ai notebook debug game mail
tool system other
```

`debug` and `tunnel` are marked in the list simply for existing: a debugger left
attached is something anyone can connect to, and a tunnel is exposing something
local to the internet right now.

### Two things worth getting right

**Only set a banner where the server truly speaks first.** SSH, SMTP, IMAP, FTP,
NATS and MySQL do. Almost no HTTP server does. A wrong banner costs a probe its
whole window waiting for bytes that will never come.

**An entry claiming a contested port must carry something else.** 3000, 8080,
8000, 5000 and 9000 belong to nobody. A signature that claims one with no
process name or HTTP fingerprint will confidently mislabel whatever else lands
there, and quarry rejects it at load.

## Adding a protocol handshake

A port is a convention; a handshake is proof. `src/handshake.rs` holds them —
each one read-only, unauthenticated, and a few bytes long. PostgreSQL's is eight
bytes asking whether TLS is available, which every PostgreSQL server answers and
nothing else does.

Add the function, name it from a signature's `probe` field, and a test asserts
every `probe` named in the shipped table actually exists.
