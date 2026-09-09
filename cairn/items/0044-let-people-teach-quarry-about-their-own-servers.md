---
id: 44
title: Let people teach quarry about their own servers
type: docs
status: done
milestone: v0.4
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: docs
effort: s
---

## Problem

However many signatures ship, the service somebody wrote last week will not be
one of them. That has to be a five-minute fix in a config file, discoverable
from the tool itself — not a pull request, and not a support question.

## Proposal

- `SIGNATURES.md`: what a signature is, every field, and how matching is scored,
  with a worked example adding a real service end to end.
- `quarry signatures` lists what is loaded and where each came from, mirroring
  `quarry themes`.
- `quarry why <port>` explains a single verdict: what matched, what it scored,
  and what it lost to. A classifier nobody can interrogate is one people argue
  with rather than fix.
- README: a short section leading with the fact that quarry knows a few hundred
  services, and that teaching it another takes four lines.

## Acceptance criteria

- [ ] every signature field documented with an example
- [ ] scoring documented, including which evidence outranks which
- [ ] `quarry why <port>` prints the evidence and the score
- [ ] the built-in signature file is a legal user signature file, so the docs can
      point at it as the worked example
