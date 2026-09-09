---
id: 43
title: Decide how far to chase container runtimes beyond Docker
type: spike
status: done
milestone: v0.4
depends_on:
- 30
created: 2026-09-08
updated: 2026-09-08
priority: p2
area: attribution
effort: s
---

## Question

0030 attributes ports to containers through the Docker API. OrbStack, Colima,
Podman, Lima, Rancher Desktop and the various local Kubernetes distributions all
publish ports too, and each does it differently. How many of these are worth
supporting directly, and is there one mechanism that covers most of them?

## Why it has to be answered before the work

It decides whether container attribution is one integration or six, and whether
`kubectl port-forward` — which is a plain process holding a port, with the
interesting information only in its arguments — belongs in the same design as
the Docker socket or somewhere else entirely.

## Options

1. **Docker API only.** OrbStack, Colima and Rancher Desktop all expose a
   Docker-compatible socket, so one integration may cover most of the field. It
   is what this machine has: `/var/run/docker.sock` is a symlink into
   `~/.orbstack`, and it answers.
2. **Docker API, plus Podman's.** Podman's socket speaks a compatible API and
   a native one; rootless Podman puts it under `$XDG_RUNTIME_DIR`.
3. **Read the command line instead.** `kubectl port-forward`, `docker run -p`
   and `ssh -L` all carry their mapping in argv, which needs no socket, no
   permission and no daemon — and is a guess rather than an answer.
4. **Everything.** Each runtime integrated separately.

## What would settle it

Check whether Colima, Podman and Rancher Desktop really do answer the same
`/containers/json`, and what socket path each uses. If they do, option 1 covers
nearly everyone for the cost of a path search, and Kubernetes port-forwards are
a separate, smaller feature that reads argv.

Worth checking on this machine first: OrbStack is installed and running, and its
Docker socket already answers.

## Answer

<!-- Filled in when the spike closes. -->

## 2026-09-08

## Answer

**Option 1 — the Docker API — but ask *every* socket, not the first that answers.**

Settled empirically on this machine, which turned out to be running two
Docker-compatible daemons at once: OrbStack behind `/var/run/docker.sock`, and
Docker Desktop behind `~/.docker/run/docker.sock`. A container started with
`docker compose` was visible to one and invisible to the other, so "first daemon
wins" reported no containers while one was plainly running.

`Containers::query` now walks every known socket path, canonicalises each to
skip a symlink and its target, and merges the results — the first daemon to
claim a port keeps it, since two runtimes cannot both publish the same one.

Paths searched: `$DOCKER_HOST`, `/var/run/docker.sock`, `~/.docker/run`,
`~/.orbstack/run`, `~/.colima/default`, `~/.rd`, and Podman's machine and
rootless sockets. All speak the same `/containers/json`, so this is one
integration rather than six.

Two things measurement caught that reasoning would not have:

- The daemon **holds the connection open** regardless of `Connection: close`,
  so reading to end-of-stream cost the full 1.5-second timeout on **every
  scan**. Reading by `Content-Length` where it is given took that under a
  millisecond.
- Asked over HTTP/1.1 the daemon replies with chunked framing. HTTP/1.0 gets a
  plain body, which is worth more here than protocol modernity.

`kubectl port-forward` stays out of scope: its mapping is in argv, not in any
API, which makes it a different and smaller feature.
