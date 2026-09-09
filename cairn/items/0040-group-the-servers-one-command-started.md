---
id: 40
title: Group the servers one command started
type: feature
status: backlog
milestone: v0.4
created: 2026-09-08
updated: 2026-09-08
priority: p2
area: attribution
effort: m
---

## Problem

`turbo dev`, `nx serve`, `foreman`, `overmind`, `pm2`, `honcho`, a Procfile or a
plain shell script routinely start five or ten servers at once. quarry shows ten
rows. They already group by repository when they share a working directory —
and fall apart when they do not, which in a monorepo is the normal case.

On this machine, four `next-server` processes under `orchard` are four separate
rows with nothing saying they came from one command and stop together.

## Proposal

Walk the process tree. Every service already carries a `ppid`; a shared ancestor
that is not the shell or `launchd` is a group. Name it after the ancestor's
command — `turbo dev`, `overmind`, `docker compose up` — and nest it inside the
repository group rather than replacing it, since both facts are true and useful.

The value is not tidiness. It is that "these nine things came from one command"
tells you they will stop together, and which single process to stop.

## Acceptance criteria

- [ ] the parent chain walked once per scan, with a depth bound
- [ ] a shared ancestor becomes a subgroup within its project
- [ ] the shell, `launchd`, `init` and the terminal are not ancestors worth
      grouping by
- [ ] the detail pane names the command that started the service
- [ ] stopping the ancestor is offered where one exists, with the same
      confirmation any other stop needs
