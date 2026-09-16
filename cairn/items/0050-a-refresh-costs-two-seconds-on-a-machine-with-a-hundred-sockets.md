---
id: 50
title: A refresh costs two seconds on a machine with a hundred sockets
type: bug
status: done
milestone: v0.4
assignee: Oddur Sigurdsson
created: 2026-09-16
updated: 2026-09-16
priority: p0
area: probe
effort: m
---

## Problem

Reported: quarry feels slow. Measured on a machine with 102 listening sockets,
a refresh took **1.8 seconds**, and it repeats every six.

Three costs, in order of size:

- **The banner window, paid 102 times.** Every silent socket was given 250ms to
  introduce itself, on every scan, forever. Twelve probe workers over 102
  sockets is 2.1 seconds of waiting. 76 of those sockets are `other` — an
  editor's IPC socket, git's file-watching daemon, a vendor's update helper —
  and none of them will ever greet.
- **The container runtime, asked every scan.** A Docker daemon on macOS lives
  behind a VM boundary and takes 20–35ms to list its containers. The rest of a
  scan is 2ms. It was asked every six seconds for an answer that changes once
  an hour.
- **Signature scoring.** 564 signatures × several patterns × three haystacks,
  per service, was 90µs each — 9ms per scan of substring searches that almost
  never matched.

## Proposal

- Give the banner window only where it can buy something: not to a service the
  table already names, and not twice to a socket that has already declined.
- Ask the runtime when the set of listening ports changes, or once a minute,
  rather than every scan.
- Reject a signature that cannot match before searching for it, with a letter
  mask compared in one `AND`.
- Raise the probe worker count. These threads are blocked on socket reads, not
  computing; the right number is set by how much waiting there is to overlap.

## Acceptance criteria

- [x] a steady-state refresh is under 50ms on a machine with a hundred sockets
- [x] the first refresh still listens to everything once
- [x] a socket that appears anew is listened to again
- [x] budgets in `tests/performance.rs` hold the line

## 2026-09-16

Measured before anything was changed, which is the only reason the right thing got fixed: the scan was never the problem. A scan was 30ms and a refresh was 1800ms, and almost all of it was one thread after another sitting in a 250ms read on a socket that was never going to say anything.

The banner window itself is not wrong — a greeting is the one thing that names a service nothing else can name, and the 250ms is measured against OrbStack's sshd at 50. What was wrong was paying it again every six seconds for the life of the process. It is paid once per socket now.

The container query was the second cost and had the same shape: 20-35ms for an answer that changes once an hour, asked every scan. Keyed on the set of listening ports, since a container cannot appear without a published port appearing.

Worker count is the smallest change and worth stating plainly: twelve was chosen as if these were CPU-bound. They are blocked on socket reads. Forty-eight.

Steady state went 1800ms to 5ms; a full refresh cycle including the scan, 1830ms to 14ms.
