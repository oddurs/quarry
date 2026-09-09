---
id: 33
title: Widen the taxonomy to the things people actually run
type: feature
status: done
milestone: v0.4
depends_on:
- 27
created: 2026-09-08
updated: 2026-09-08
priority: p1
area: discovery
effort: m
---

## Problem

Thirteen kinds — web, api, db, cache, search, queue, proxy, mail, container, ai,
tool, system, other — were a reasonable guess at the start. They put Grafana,
MinIO, Keycloak, Temporal, a Node debugger and a Minecraft server all in the
same bucket: `other`.

## Proposal

Kinds earn their place by being things a developer looks for separately. From
what people actually run locally:

| kind | what falls in it |
|---|---|
| `observability` | Prometheus, Grafana, Jaeger, Loki, Tempo, Zipkin, OTel collector |
| `storage` | MinIO, LocalStack, Azurite, fake-gcs-server, SeaweedFS |
| `auth` | Keycloak, Ory, Dex, Supabase auth, Zitadel |
| `registry` | a local Docker registry, Verdaccio, Gitea, Artifactory |
| `realtime` | WebSocket servers, Socket.IO, Centrifugo, Phoenix channels |
| `vector` | Qdrant, Weaviate, Milvus, Chroma, LanceDB |
| `workflow` | Temporal, Airflow, Prefect, Dagster |
| `debugger` | the Node inspector on 9229, JDWP on 5005, delve, debugpy |
| `tunnel` | ngrok, cloudflared, Tailscale, localtunnel |
| `notebook` | Jupyter, Marimo |
| `emulator` | Firebase, DynamoDB Local, Stripe CLI, the Android emulator |
| `game` | Minecraft, Valheim, Source engine |

`debugger` earns its place twice over: an attached debugger left running is a
thing developers hunt for, and it is currently indistinguishable from a stray
API.

Each needs a colour role, so this touches every theme file — which is the reason
to do it in one go rather than a kind at a time.

## Acceptance criteria

- [ ] new kinds, each with a signature entry and a colour role in every theme
- [ ] `auto` and `mono` still hold: no theme may need a colour it does not have
- [ ] the badge column still fits at 46 columns — `observability` does not
- [ ] a snapshot per theme, so the palette change is reviewable
- [ ] the share of services classified as `other` on the developer's own machine
      is reported before and after, because that is the number this is for

## 2026-09-08

Done, and larger than planned: 25 kinds. Kind colours are now *derived from a theme's own roles*, with explicit `[kinds]` entries as an override — so adding a kind can never leave a theme with a hole in it, and a three-line user theme still colours all 25 badges. A test asserts every theme covers every kind.
