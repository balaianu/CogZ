# Consolidation

CogZ continuously consolidates its memory: deduplicates entries, detects contradictions, promotes supported observations to rules, and merges confirmed duplicates. This document describes each phase.

## When consolidation runs

| Phase | When | Triggered by |
|---|---|---|
| Dedup | On insert | Every `record_observation`, `create_rule`, `create_knowledge` call |
| Contradiction | On insert | Every observation/rule insert (when NLI model available and `contradiction_check = true`) |
| Promotion | Background | `cogz consolidate` CLI, `consolidate` MCP tool, or `session_end` hook |
| Merge | Background | `cogz consolidate` CLI, `consolidate` MCP tool, or `session_end` hook |

Dedup and contradiction detection are synchronous — they run on every insert and return warnings to the caller. Promotion and merge are deferred — they run on demand or at session end.

## Dedup

Two checks run on every insert:

1. **Title match** — exact (case-insensitive) or fuzzy (Levenshtein-based, threshold `title_match_threshold` = 0.85). Returns a `DuplicateWarning` with a suggestion. Works without embeddings.

2. **Embedding similarity** — cosine similarity from KNN L2 distance. If similarity > `dedup_threshold` (0.85), sets `dedup_flagged`. Requires the embedding model to be available.

Both checks degrade gracefully: without embeddings, only title match runs. Without any existing entities, no checks run.

Dedup flags duplicates but does not merge them. Merge is a separate background step (below).

## Contradiction detection

When `contradiction_check = true` and the NLI model is available, new observations and rules are checked against existing entities of the same type.

**Pipeline:**
1. Fetch candidates — existing entities of the same type with high embedding similarity (cosine > `contradiction_cosine_threshold` = 0.85) and text length ratio < `contradiction_length_ratio` = 5.0.
2. Classify with NLI — for each candidate, the NLI model computes P(entailment), P(neutral), P(contradiction).
3. Flag contradictions — if P(contradiction) > `contradiction_threshold` (0.70), the pair is flagged.

Flagged pairs get a `contradicts` edge in the DB and frontmatter. The new entity is not rejected — it's created with the contradiction flagged for review.

When the NLI model is unavailable, contradiction detection is skipped. Dedup still runs via title match.

## Promotion

An observation with enough supporting observations (count ≥ `promotion_threshold` = 3) is promoted to a rule.

**How it works:**
1. Find candidates — active observations with ≥ `promotion_threshold` `supports` edges from other active observations.
2. For each candidate, create a new rule file with:
   - `derived_from` edge to the source observation
   - `supporting_ids` listing all supporters
   - Content copied from the observation
3. The source observation is not mutated — promotion is additive.

Promoted rules are git-tracked and reviewable. The `derived_from` edge provides traceability from the rule back to the observation that generated it.

## Merge

Confirmed duplicate observations are merged: one entity survives, the other is marked `superseded` with a `superseded_by` field.

**Pipeline:**
1. Find candidates — pairs of active observations with embedding cosine similarity > `dedup_threshold` (0.85).
2. NLI confirmation — bidirectional entailment check. Both P(A entails B) and P(B entails A) must exceed `dedup_nli_threshold` (0.85). True duplicates entail mutually; a subset-fact does not.
3. If NLI is unavailable, candidates are reported but not merged (conservative — no merge without confirmation).
4. For each confirmed pair:
   - Survivor = earlier-created entity
   - Superseded = later-created entity
   - Update superseded entity's frontmatter: `status: superseded`, `superseded_by: <survivor_id>`
   - Redirect all edges from the superseded entity to the survivor
   - Record a `knowledge_merged` event

The superseded entity's file is kept on disk with `status: superseded`. Graph expansion follows the `superseded_by` edge to reach the survivor.

**Knowledge is flagged, not auto-merged.** Knowledge is human-curated. Near-duplicate knowledge pairs are reported by `cogz doctor` for manual review.

**Rules are surfaced for review, not auto-merged.** Same rationale.

## Config

All consolidation thresholds are in the `[consolidation]` section of `config.toml`. See [Configuration](../configuration.md#consolidation) for the full reference.

## See also

- [Architecture](architecture.md) — system overview
- [Entity Model](entity-model.md) — state machine and update policy
- [Configuration](../configuration.md) — consolidation config section
