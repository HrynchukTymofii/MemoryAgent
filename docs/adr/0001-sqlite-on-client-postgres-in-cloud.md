# ADR-0001: SQLite on the client, Postgres in the cloud

**Status:** Accepted · **Date:** 2026-09-06 · **Supersedes:** spec sections 13, 14

## Context

Spec section 13 names PostgreSQL + pgvector as the primary database and section
14 adds Redis, with section 33 placing both on the end user's machine. The
product is also intended to be sold to non-technical customers.

## Problem

Shipping Postgres and Redis to consumer Windows machines fails on install, not
on capability:

- Postgres needs an installer, a Windows service, a TCP port, roughly 250 MB,
  and a migration that must not fail on every app update. Docker Desktop is not
  an acceptable prerequisite for a paying consumer.
- Redis has no official Windows build. Only Memurai or WSL, both of which are
  additional installs.
- Every one of these is a support ticket, and each one happens at the worst
  possible moment: before the user has seen the product work.

The distinction that resolved this is **not** local versus server. It is
**developer machine versus customer machine.** Running Postgres locally during
development is trivial and stays.

## Decision

**Client:** SQLite in WAL mode, with FTS5 for keyword search and `sqlite-vec`
for vector search. One file, no service, no port, no installer step.

**Cloud (paid sync tier):** PostgreSQL + pgvector + Redis + LangGraph, developed
locally via `docker-compose` and deployed to a server later.

Redis's three jobs on the client are replaced in-process:

| Redis role | Client replacement |
|------------|--------------------|
| task queue / background jobs | `jobs` table in SQLite + Tokio workers |
| cache | in-memory maps behind `parking_lot` |
| pub/sub | Tauri's event bus |

## Consequences

**Good**

- Installer drops from ~1 GB to ~150 MB; cold start from seconds to <500 ms.
- FTS5 and `sqlite-vec` live in the same engine, so hybrid retrieval (spec 11)
  is one SQL query in one process with no IPC — this is *faster* than the
  Postgres design, not a compromise.
- The database is a single file: backup, sync and support-debugging all become
  trivial.

**Bad**

- `sqlite-vec` brute-forces the vector scan. Fine to roughly 100k items; beyond
  that we add IVF partitioning or shard by collection. Revisit at 50k items.
- Two schemas to keep aligned (SQLite client, Postgres cloud). Mitigated by
  generating both from one schema definition and testing the sync contract.

**Neutral**

- The Postgres/pgvector/Redis learning goal from spec section 40 is fully
  preserved — it simply lives in `cloud/`, where that stack belongs.
