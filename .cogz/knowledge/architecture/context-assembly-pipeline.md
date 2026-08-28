---
id: b3c4d5e6-0001-4aaa-bbbb-000000000002
title: Context assembly pipeline
type: knowledge
status: active
created_at: 2026-08-28T19:40:00Z
updated_at: 2026-08-28T19:40:00Z
references: ["ac43ee1b-2322-4ba1-afcf-cd2464a2d065"]
category: architecture
tags: ["context", "phase-6", "modes", "token-budget"]
---

The context assembly layer sits on top of search and produces
`ContextPack` — the primary output of CogZ for agent consumption.

## Module structure

- `src/context/mod.rs` — `ContextPack`, `ContextSection`,
  `PackMetadata` types
- `src/context/modes.rs` — `ContextMode` enum (cold_start, task,
  escalation) with serde support
- `src/context/assemble.rs` — `assemble_context()` orchestrator
- `src/context/compress.rs` — token estimation, priority sorting,
  budget fitting

## Three modes

1. **cold_start** — no query needed. Fetches recent rules and
   observations directly from the DB via `get_entities_by_type`.
   No search, no graph expansion. Compact pack for session start.

2. **task** — query-scoped. Runs full search (FTS + optional
   vector) with graph expansion. Results become context sections
   with graph paths as provenance.

3. **escalation** — wider task mode. More search results
   (`escalation_max_results`), more graph hops
   (`escalation_max_hops`). Used when task pack was insufficient.

## Token budgeting

The chars/4 heuristic (`estimate_tokens`) approximates token
count. Sections are sorted by source priority (rules >
observations > knowledge > code), then by relevance. The
`fit_budget` function adds sections in priority order until the
budget is exhausted. The last fitting section is truncated to
the remaining budget (accounting for title token cost). All
subsequent sections are dropped and listed in
`metadata.dropped_sources`.

## Config

The `[context]` section in `config.toml` controls all defaults:
token budget, cold_start limits, task/escalation search
parameters. Added in Phase 6 — not in the original architecture
doc's config section, but required by the MCP contract's
`max_tokens` "default: from config" specification.
