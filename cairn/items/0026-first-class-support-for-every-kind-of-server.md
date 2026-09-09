---
id: 26
key: v0.4
title: First-class support for every kind of server
type: milestone
status: backlog
created: 2026-09-08
updated: 2026-09-08
priority: p2
---

quarry sees listening TCP sockets, guesses what they are from a port table and a
process name, and asks HTTP if it thinks they might answer. That covers dev
servers well and almost nothing else well.

Measured on one developer machine:

- 26 TCP listeners visible; **107 + 36 unix domain sockets from Docker alone**,
  plus node, wrangler, OrbStack and 1Password — none of it visible at all.
- Port 32222 announces `SSH-2.0-OrbStack` the instant you connect. quarry shows
  it as `other · open` and has nothing else to say.
- A container is running. Its published ports are attributed to the OrbStack
  process, not to the container, its image, or the repository it was built from.
- Five services show as `other · open`: quarry connected, learned nothing, and
  reported that.

"First class" means four things, in order: **see it**, **identify it**, **know
whose it is**, and **know whether it is healthy**. Today only the first is done
well, and only for TCP.

Done when a developer can run the things developers actually run — a database, a
container, a queue, a debugger, a tunnel — and quarry names each one correctly
without being told.
