---
id: 30
title: Attribute a container's ports to the container
type: feature
status: done
milestone: v0.4
created: 2026-09-08
updated: 2026-09-08
priority: p0
area: attribution
effort: l
---

## Problem

A published container port belongs to the host process that publishes it. On
this machine, ports 32222 and 62723 are attributed to `OrbStack` — a container
is running, and quarry says the VM's name instead of the container's, the
image's, or the project's.

This is the single most misleading thing quarry currently reports. Anyone
running their stack in Compose sees one row per published port, all of them
labelled after the runtime, none of them connected to the repository they came
from.

## Proposal

Ask the daemon. `/var/run/docker.sock` speaks plain HTTP over a unix socket with
no client library and no dependency — `GET /containers/json` returns names,
images, published ports and labels, and it answers on this machine today.

Compose writes the labels that make attribution work:

- `com.docker.compose.project` — the group these containers belong to
- `com.docker.compose.service` — what to call this one
- `com.docker.compose.project.working_dir` — **the directory on disk**, which
  feeds straight into the existing repository resolver

So a container in a Compose project resolves to the same repository as a process
started from that directory, and the two group together — which is what the user
means by "my project".

## Acceptance criteria

- [ ] published ports map to container name, image and state
- [ ] a Compose project becomes a project group, resolved through its working
      directory to a repository where there is one
- [ ] a container that is unhealthy or restarting reads as unhealthy, using the
      daemon's own health status rather than a guess
- [ ] no daemon, or no permission, degrades to today's behaviour silently
- [ ] the daemon is asked once per scan, not once per port
- [ ] the detail pane shows image, container name and Compose service

## 2026-09-08

Done, verified end to end against a real Compose stack: a published port that read as `Docker / Data` now reads as `quarry-compose-test / cache`, grouped under its Compose project and resolved through `com.docker.compose.project.working_dir` to a repository where there is one. The daemon's own health verdict is used rather than a guess, and its answer outranks both the signature table and anything a probe hears — a container's identity is not something to re-derive from the host process that published its port.
