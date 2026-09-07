# Search

CogZ uses hybrid FTS5 + vector search with Reciprocal Rank Fusion (RRF) and graph expansion. This document describes the search pipeline.

## Pipeline

```
Query
  → embed query (knowledge model + code model, if available)
  → FTS5 search (always available)
  → KNN vector search (knowledge_embeddings + code_embeddings, if available)
  → RRF fusion (combine FTS + vector results)
  → Graph expansion (BFS from matched entities)
  → Return ranked results with graph provenance
```

## FTS5 search

SQLite FTS5 with `porter unicode61` tokenizer. Searches the `entities_fts` virtual table, which is synced to the `entities` table via triggers (insert/update/delete).

FTS5 is always available, even without models. It provides lexical matching — exact term and phrase matches. It is the fallback when vector search is unavailable.

## Vector search

sqlite-vec provides KNN search over embedding vectors. Two separate vec0 tables:

- `code_embeddings` — embeddings from CodeRankEmbed (code entities)
- `knowledge_embeddings` — embeddings from bge-base (knowledge entities)

Code and knowledge use different embedding models with incompatible vector spaces even at the same dimensionality. Separate tables ensure KNN only compares vectors within the same space.

The query is embedded with both models (when available), producing two query embeddings. Each is used for a KNN search in its respective table.

## RRF fusion

Reciprocal Rank Fusion combines ranked lists from multiple sources without needing score calibration:

```
score(entity) = Σ  1 / (rrf_k + rank_in_source)
```

Where `rrf_k` is a smoothing constant (default: 60). Lower values produce sharper ranking.

**Sources and weights** (from `[search]` config):

| Source | Weight | When |
|---|---|---|
| FTS5 | `fts_weight` (0.3) | Always |
| Knowledge vector | `vec_weight` (0.4) | Knowledge model available |
| Code vector | `code_vec_weight` (0.3) | Code model available |

When a source is unavailable (no model), its weight is redistributed to the remaining sources.

## Source balancing

By default (`source_balance_enabled = false`), a fixed 0.5/0.5 split is used between code and knowledge vector results. A fixed split outperforms balance detection on overall retrieval quality.

When `source_balance_enabled = true`, the system detects code vs knowledge query intent using two signals:
1. **KNN distance spread** — a strong match produces a distance gradient; a weak match produces uniform distances.
2. **FTS pool size ratio** — a code query matches more code entities lexically than knowledge entities.

`min_source_proportion` (default: 0.2) ensures neither source is completely suppressed — each gets at least 20% of the RRF weight.

## Graph expansion

After RRF fusion, the top results are used as seeds for BFS graph expansion. The expansion follows all edge types (`references`, `auto_references`, `calls`, `imports`, `extends`, `contains`, `supports`, `derived_from`, `superseded_by`, `contradicts`).

Expanded entities get a decayed relevance score: `seed_relevance * 0.5^hops`. This ensures direct matches rank higher than graph-expanded results.

Each result includes a `graph_path` — the list of entity IDs from the seed to this entity — and a human-readable `graph_path_description`.

**Max hops** is configurable per mode:
- Task mode: `task_max_hops` (default: 2)
- Escalation mode: `escalation_max_hops` (default: 3)

Use `--no-expand` on the CLI or `expand: false` in the MCP tool to disable expansion.

## Search modes

The `search_mode` field in results indicates how search was executed:

| Mode | FTS | Knowledge vector | Code vector | When |
|---|---|---|---|---|
| `hybrid` | yes | yes | yes | Both models available |
| `knowledge_hybrid` | yes | yes | no | Code model unavailable |
| `code_hybrid` | yes | no | yes | Knowledge model unavailable |
| `fts_only` | yes | no | no | No models available |

## Code search

The `--code` flag (CLI) or `code_search: true` (MCP) uses the CodeRankEmbed model for query embedding instead of the knowledge model. CodeRankEmbed requires a query prefix: `"Represent this query for searching relevant code: "` prepended to the query. This is applied automatically.

Use `--code` for queries about code structure, function behavior, or implementation details. Use the default (knowledge model) for queries about concepts, decisions, or documentation.

## Known limitations

1. **Graph expansion dilutes precision.** Graph-expanded entities are technically related but may not contain the query keywords. The decayed relevance score mitigates this but doesn't eliminate it.
2. **Code/knowledge embedding mismatch.** When a code query is embedded with the knowledge model (or vice versa), the KNN search may miss relevant results. Using `--code` for code-focused queries helps.
3. **No relevance threshold.** All results up to `max_results` are returned regardless of score. A threshold would filter low-relevance graph-expanded noise but risks missing valid results.

## See also

- [Architecture](architecture.md) — system overview
- [Configuration](../configuration.md) — search config section
- [Degradation](degradation.md) — FTS-only mode
- [Evaluations](../evaluations/) — retrieval benchmarks and resource profiles
