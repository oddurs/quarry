---
id: 36
title: Ask the health endpoint a service actually has
type: feature
status: done
milestone: v0.4
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: probe
effort: s
---

## Problem

quarry asks for `/` and takes the answer at face value. Plenty of healthy
services return `404` there, because `/` is not where they live — and the config
file lets a user say so per port, which means knowing in advance and writing it
down for every service.

Most of these are conventions with names. quarry knows what the service is; it
should know where to ask.

## Proposal

Put the health path in the signature, so it comes with the identification:

| service | path |
|---|---|
| Spring Boot | `/actuator/health` |
| Prometheus | `/-/healthy` |
| Grafana | `/api/health` |
| Elasticsearch | `/_cluster/health` |
| Kubernetes-shaped | `/healthz`, `/readyz` |
| MinIO | `/minio/health/live` |
| Temporal | `/health` |

Where no signature matches, `/` remains the guess, and the `[health]` config
section still overrides everything — a user's own service is a case no table
will ever cover.

## Acceptance criteria

- [ ] health path resolves signature-first, config over everything
- [ ] the detail pane says which path was asked, so a wrong guess is visible
- [ ] a health path that 404s falls back to `/` rather than reporting a failure
      the service did not have
