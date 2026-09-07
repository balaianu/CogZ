# Graceful Degradation

CogZ is designed to function without ONNX models. This document describes what works, what doesn't, and how the system degrades.

## FTS-only mode

When no models are available (not downloaded, download disabled, or insufficient RAM), CogZ operates in FTS-only mode. This is the baseline — everything else is an enhancement.

### What works in FTS-only mode

| Feature | Available | Notes |
|---|---|---|
| `cogz init` | yes | No models needed |
| `cogz index` | yes | Files sync without embeddings; code entities indexed without vector embeddings |
| `cogz reindex` | yes | Incremental sync works |
| `cogz search` | yes | FTS5 search only, no vector results |
| `cogz context` | yes | FTS-only retrieval, no vector ranking |
| `cogz status` | yes | Reports models as unavailable |
| `cogz consolidate` | yes | Title-based dedup only; no NLI contradiction detection; no NLI-confirmed merge |
| `cogz capture-event` | yes | Use `--fts-only` flag for fast hook calls |
| `cogz doctor` | yes | All checks work; reports models as unavailable |
| `cogz doctor --prune-observations` | yes | Pruning works |
| `cogz mcp-stdio` | yes | All 13 tools work; search returns FTS-only results |
| All MCP tools | yes | `search_mode` reports `fts_only` |
| All hooks | yes | Use `--fts-only` for speed |

### What doesn't work in FTS-only mode

| Feature | Unavailable | Reason |
|---|---|---|
| Vector search | no KNN results | No embedding models |
| Embedding-based dedup | no similarity check | No embedding models |
| NLI contradiction detection | no contradiction flagging | No NLI model |
| NLI-confirmed merge | candidates reported but not merged | No NLI model; conservative — no merge without confirmation |
| Code embeddings | code entities have no vectors | No code model |

## Model unavailability

Models can be unavailable for several reasons:

1. **Not downloaded.** `auto_download = false` or `--no-download` was used. Run `cogz models download` to fetch them.

2. **Insufficient RAM.** Available RAM is below `model_min_free_mb` (default: 512 MB). The system degrades to FTS-only rather than loading a model that would cause swapping.

3. **ONNX Runtime missing.** The `ort` crate loads the ONNX Runtime dynamically. If the library is not found, models can't load. The runtime is auto-downloaded on first use on supported platforms.

4. **Model files corrupt.** `clean_broken_cache()` runs before every model load, removing `.incomplete` files >1h old and empty `refs/main` files. If the model files are corrupt after cleanup, loading fails gracefully.

In all cases, the system logs a warning, sets `available: false` in status responses, and continues with FTS-only capability. No panics.

## Model lifecycle

Models are lazy-loaded — creating the model object is cheap; the actual ONNX runtime loads on first `embed()` or `classify()` call.

**Auto-unload:** After `model_idle_ttl` seconds (default: 300 = 5 minutes) of inactivity, models are unloaded from memory, freeing ~300–500 MB. RAM drops back to ~11 MB when all models are unloaded.

**Auto-download:** When `auto_download = true` (default), `cogz index` fetches models from HuggingFace on first use. `--no-download` or `auto_download = false` disables this. The system still functions in FTS-only mode.

**Partial availability:** If only the code model is available (not knowledge), search runs in `code_hybrid` mode (FTS + code vector). If only knowledge is available, search runs in `knowledge_hybrid` mode. The `search_mode` field in results indicates which mode was used.

## Hook-specific degradation

Hooks should always use `--fts-only` to avoid loading the ONNX runtime on every call. See [Hooks](../integration/hooks.md) for timing details and rationale.

## Resource profile

| State | RAM | Disk |
|---|---|---|
| Binary only (no models, no DB) | ~11 MB | ~8 MB |
| FTS-only mode (DB loaded, no models) | ~30 MB | ~50 MB |
| All models loaded | ~300–500 MB | ~550 MB |
| Models loaded then unloaded (idle) | ~11 MB | ~550 MB |

See [Evaluations](../evaluations/) for the full resource consumption profile.

## See also

- [Architecture](architecture.md) — system overview
- [Configuration](../configuration.md) — embedding config section
- [Evaluations](../evaluations/) — resource profile and benchmarks
