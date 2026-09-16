---
id: 40
title: Group the servers one command started
type: feature
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
created: 2026-09-08
updated: 2026-09-15
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

- [x] the parent chain walked once per scan, with a depth bound
- [x] a shared ancestor becomes a subgroup within its project
- [x] the shell, `launchd`, `init` and the terminal are not ancestors worth
      grouping by
- [x] the detail pane names the command that started the service
- [x] stopping the ancestor is offered where one exists, with the same
      confirmation any other stop needs

## 2026-09-15

Walking the chain needed process information quarry does not otherwise load: only listening pids are refreshed, and an ancestor is by definition not one. So each generation is fetched as a batch — at most eight refreshes of a shrinking set, rather than one lookup per process.

Two judgements. A launcher that started one service within a group is not a subgroup: it says nothing the service did not already say, so the row is only emitted where the count is above one. And the furniture list — shells, terminals, launchd, init, containerd-shim, the editors that spawn integrated terminals — is what keeps the whole machine out of one bucket, since those are ancestors of everything.

Verified against a real case rather than only fixtures: two http.servers started by one python3 process in this repo produced exactly the intended shape, a launcher row named after the command with the count beside it, nested inside the project group. On this machine without that, nothing groups — which is correct, because no project here has two services from one command.
