---
id: 51
title: Prefer the repository's own name, and stop inventing projects from folders
type: feature
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
created: 2026-09-16
updated: 2026-09-16
priority: p1
area: attribution
effort: s
---

## Problem

Three places where quarry showed a worse name than it had available.

**A checkout's directory is not the repository's name.** `~/Code/astralia` is a
clone of `oddurs/cairn`; `~/Code/NetherC` is a clone of `oddurs/nether-c`. The
group was named after the directory, so the project answered to a name nobody
else uses for it.

**A directory is not a project.** With no repository, quarry grouped by the
working directory, which produced groups called `Data`, `Code`, `Helpers` and
`Application` — the innards of application bundles, named after nothing anybody
is working on.

**A container's name is half the time Docker's, not a person's.** Seven
containers on this machine are called `beautiful_heisenberg`, `goofy_diffie`
and so on, all running `rust:1-slim`, and not one of the names says what any of
them is doing.

## Proposal

- The repository's name where there is a remote to read it from; the directory
  only when there is not.
- No fall back to the working directory for grouping. The directory is still
  reported in the detail pane, where saying where something runs is the point.
- For a container: the name somebody meant — the Compose service name, or one
  given with `--name` — and failing that, what the image says it is running.

## Acceptance criteria

- [x] a clone in a differently-named directory groups under the repository
- [x] a repository with no remote keeps its directory name
- [x] every worktree of one project answers to the same name
- [x] a folder no longer names a project, and is still shown in the detail pane
- [x] a container Docker named is shown by its image
- [x] a container somebody named keeps that name

## 2026-09-16

Container naming came out better than the version first agreed. The plan was to show the repository name for a container in a repository, which would have made eleven containers in one stack eleven rows all reading the same word. What was wanted was for them to say what they are, so the rule became: the name somebody meant first — the Compose service name, or one given with --name — and failing that the image.

Detecting a name Docker invented is by shape rather than by shipping both of its word lists: two lowercase words of three or more letters joined by an underscore. That is close to exclusive, since Compose uses hyphens and so does almost everyone naming by hand, and the boundary is tested — my_app survives because 'my' is two letters, web_server does not.

The image beats a signature derived from it. nginx:alpine matched 'Nginx Proxy Manager', which is different software that happens to have nginx in its name; the image is the fact and the signature is a guess made from it.
