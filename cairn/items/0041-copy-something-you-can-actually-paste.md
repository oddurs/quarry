---
id: 41
title: Copy something you can actually paste
type: feature
status: done
milestone: v0.4
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-08
priority: p2
area: chrome
effort: s
---

## Problem

`y` copies `http://localhost:5432` for a PostgreSQL server. There is nothing to
do with that. The same for Redis, MongoDB, a unix socket, and every non-HTTP
service quarry can see.

Pressing enter on one opens a browser at a page that will never load.

## Proposal

The URI belongs in the signature, as a template:

```
postgres://localhost:{port}/
redis://localhost:{port}
mongodb://localhost:{port}
amqp://localhost:{port}
unix:{path}
```

Then `y` copies the thing you would paste into a client, and `enter` opens the
browser only where a browser is the right answer — for everything else it copies
instead, and says so, rather than launching something that cannot work.

## Acceptance criteria

- [ ] URI templates in signatures, rendered with the real port or path
- [ ] `y` copies the URI for the service's kind
- [ ] `enter` on a non-HTTP service copies rather than opening, with a toast
      that says which
- [ ] the detail pane shows the URI it would copy
