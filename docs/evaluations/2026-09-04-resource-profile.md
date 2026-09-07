# CogZ Resource Consumption Profile

**Date:** 2026-09-04
**Repository under test:** CogZ itself (1276 entities, 3906 edges, 1276 embeddings)

## Reference Machine

| Property | Value |
|---|---|
| Architecture | x86_64 |
| Cores/Threads | 8 cores / 16 threads |
| RAM | 32 GB |
| OS | Linux |
| Page size | 4096 bytes |

RAM measurements below are from this machine. Disk and algorithmic
metrics are portable. RAM values include OS-level overhead (page cache,
shared library mapping, allocator fragmentation) that varies by platform.
The fixed components (model weights, embedding vectors, SQLite cache)
are listed separately so readers can compute expected RAM on any machine.

## Quick Reference

| What | Value |
|---|---|
| Total install size | 515 MB (binary + runtime + 3 models + DB) |
| MCP idle RAM | 11 MB (models unloaded) / 820 MB (after first search) |
| Hook overhead | <20 MB RAM, sub-second, no models loaded |
| Context pack cost | 2,000-7,100 tokens (cold start: ~2K, task: ~5-7K) |
| Full index | ~12s foreground + ~2 min background, ~1 GB peak RAM |
| Per-repo DB | ~15 MB for ~1300 entities, scales linearly |

---

## 1. Static Disk Resources (Portable)

### Binary

| Component | Size |
|---|---|
| `cogz` release binary | 23.3 MB |

### Database

| Component | Size | % of DB |
|---|---|---|
| **Total DB** | **14.6 MB** | 100% |
| code_embeddings (vec0) | 6.3 MB | 43% |
| knowledge_embeddings (vec0) | 3.1 MB | 22% |
| entities table | 2.7 MB | 18% |
| entities_fts (FTS5 index) | 0.6 MB | 4% |
| edges table + indexes | 1.2 MB | 8% |
| events + entity_access + meta | 0.1 MB | 1% |

**Scaling formula:** DB size ≈ (entities × 2.1 KB) + (embeddings × 7.8 KB)
+ (edges × 0.13 KB). The vec0 overhead is ~2x the raw embedding bytes
due to chunking metadata and rowid indexes.

### Embedding storage math

| Property | Value |
|---|---|
| Dimension | 768 (f32 = 4 bytes) |
| Raw bytes per embedding | 3,072 bytes (3 KB) |
| Code embeddings | 1206 × 3 KB = 3.5 MB raw |
| Knowledge embeddings | 70 × 3 KB = 210 KB raw |
| sqlite-vec overhead | ~5.3 MB (chunking, metadata, rowids) |
| Total embedding storage in DB | 9.4 MB |

### Models on disk

| Model | Role | Disk size | Dimension |
|---|---|---|---|
| CodeRankEmbed-int8 | Code embedding | 139 MB | 768 |
| bge-base-en-v1.5 | Knowledge embedding | 218 MB | 768 |
| nli-deberta-v3-xsmall | NLI contradiction | 96 MB | N/A |
| **Total models** | | **453 MB** | |

### ONNX Runtime

| Component | Size |
|---|---|
| libonnxruntime.so | 23.7 MB |

### Total disk footprint (first install)

| Component | Size |
|---|---|
| Binary | 23 MB |
| ONNX Runtime | 24 MB |
| Models (all 3) | 453 MB |
| DB (this repo) | 15 MB |
| **Total** | **515 MB** |

---

## 2. RAM Profile (Reference Machine)

### MCP Server

| State | RSS | Delta | Loaded models |
|---|---|---|---|
| Idle (initialized, no search) | 11 MB | — | none |
| After first search | 820 MB | +809 MB | knowledge + code |
| After record_observation | 769 MB | +758 MB | knowledge only |
| Steady state (models idle, TTL=300s) | 820 MB | — | knowledge + code |
| After idle TTL expires (5 min) | ~11 MB | -809 MB | none (unloaded) |

**Note:** The MCP server loads both embedding models on the first
search call regardless of the `code` flag. Both `embed_query_for_search`
calls execute before the search runs. Subsequent searches reuse loaded
models. The NLI model loads lazily only when contradiction candidates
exist during a write operation.

### CLI Commands (peak RSS, process exits after each)

| Command | Peak RSS | Models loaded | Why |
|---|---|---|---|
| `cogz status` | 11 MB | none | DB queries only |
| `cogz search --fts-only` | 7 MB | none | FTS5 only |
| `cogz search` (hybrid) | 595 MB | knowledge | Query embedding |
| `cogz search --code` | 242 MB | code | Query embedding |
| `cogz context cold_start` | 13 MB | none | DB queries only |
| `cogz context task` | 595 MB | knowledge | Query embedding |
| `cogz doctor` | 13 MB | none | DB queries only |
| `cogz consolidate --dry-run` | 16 MB | none | DB queries only |
| `cogz consolidate` | 16 MB | none* | *NLI only if candidates exist |
| `cogz index` (foreground) | 973 MB | both | Embed 70 knowledge entities inline |
| `cogz index` (background embed) | 679 MB | code | Embed 1206 code entities |
| `cogz reindex` | 13 MB | none | Incremental, no changes |

### Hook Events (peak RSS, `--fts-only` flag)

| Hook | Peak RSS | Models loaded | Output size |
|---|---|---|---|
| session_start | 13 MB | none | 9.4 KB |
| prompt_submit | 16 MB | none | 33.7 KB |
| file_save | 18 MB | none | 0.1 KB |
| session_end | 16 MB | none | 0.1 KB |
| stop | 10 MB | none | 0.1 KB |

All hooks use `--fts-only` by design, avoiding ONNX model loading.
This keeps hook calls under 20 MB RSS and sub-second latency.

### Model RAM cost breakdown

| Model | Disk size | RAM (CLI) | RAM (MCP) | Ratio |
|---|---|---|---|---|
| ONNX Runtime (shared) | 24 MB | ~50 MB | ~50 MB | 2.1x |
| bge-base (knowledge) | 218 MB | ~588 MB | ~809 MB | 2.7-3.7x |
| CodeRankEmbed (code) | 139 MB | ~235 MB | — | 1.7x |
| Both models (CLI index) | 357 MB | ~966 MB | — | 2.7x |
| Both models (MCP search) | 357 MB | — | ~809 MB | 2.3x |

**Why RAM > disk:** ONNX Runtime creates an optimized computation
graph in memory, allocates arena memory for intermediate tensors, and
memory-maps the model file. The `with_memory_pattern(false)` setting
prevents unbounded arena growth. Expect 1.7-3.7x disk size in RAM.

### Idle TTL behavior

Config: `model_idle_ttl = 300` (5 minutes), `model_min_free_mb = 512`.

After 5 minutes without a search/context call, the MCP server unloads
all ONNX sessions and returns to ~11 MB RSS. The next search reloads
them (~1s overhead). The `has_enough_memory` check prevents loading
if free RAM drops below 512 MB — search degrades to FTS-only instead.

---

## 3. Algorithmic Costs per Execution Path (Portable)

### SQL query complexity

| Path | SQL queries | File I/O | ONNX calls |
|---|---|---|---|
| **CLI: status** | ~6 (counts, model check) | 0 | 0 |
| **CLI: search --fts-only** | 3 (FTS + entity batch + graph) | 0 | 0 |
| **CLI: search hybrid** | 4 (FTS + KNN + entity batch + graph) | 0 | 1 (query embed) |
| **CLI: search --code** | 4 (FTS + KNN + entity batch + graph) | 0 | 1 (query embed) |
| **CLI: context cold_start** | ~5 (rules + obs + code map) | 0 | 0 |
| **CLI: context task** | 5+ (search + assembly) | 0 | 1 (query embed) |
| **CLI: context escalation** | 5+ (wider search + assembly) | 0 | 1 (query embed) |
| **CLI: index** | N inserts (entities + edges + FTS) | 70 reads + 1206 source reads | 1 batch (70 knowledge) + spawn bg |
| **CLI: reindex** | incremental (hash-skipped) | 70 reads (hash check) | 0 if no changes |
| **CLI: reset** | 1 (DROP table) | 0 | 0 |
| **CLI: doctor** | ~10 (integrity + dedup + stale) | 0 | 0 |
| **CLI: consolidate** | candidate queries + N writes | 0-N file writes | 0-1 (NLI if candidates) |
| **CLI: update** | 0 | 0 (temp only) | 0 |
| **MCP: get_status** | ~6 | 0 | 0 |
| **MCP: search** | 4 | 0 | 2 (query + code embed) |
| **MCP: get_context** | 5+ | 0 | 2 (query + code embed) |
| **MCP: record_observation** | 3+ (sync + dedup + event) | 1 write + 1 read | 1-2 (embed + NLI) |
| **MCP: create_rule** | 3+ (sync + dedup + event) | 1 write + 1 read | 1 (embed) |
| **MCP: create_knowledge** | 3+ (sync + dedup + event) | 1 write + 1 read | 1 (embed) |
| **MCP: update_knowledge** | 3+ (sync + re-embed + event) | 1 write + 1 read | 1 (re-embed) |
| **MCP: query_observations** | 1 | 0 | 0 |
| **MCP: query_rules** | 1 | 0 | 0 |
| **MCP: query_knowledge** | 1 | 0 | 0 |
| **MCP: list_entities** | 1 | 0 | 0 |
| **MCP: consolidate** | candidate + N write queries | 0-N file writes | 0-1 (NLI) |
| **MCP: capture_event** | 1+ (event + context) | 0 | 0 (FTS-only) |
| **Hook: session_start** | ~6 (event + cold_start) | 0 | 0 |
| **Hook: prompt_submit** | 4+ (event + FTS search + assembly) | 0 | 0 |
| **Hook: file_save** | 3+ (event + sync + stale check) | 1 read + 1 write | 0 |
| **Hook: session_end** | 2+ (event + consolidate) | 0-N | 0-1 (NLI) |
| **Hook: stop** | 1 (event) | 0 | 0 |

### ONNX inference tensor sizes (portable)

| Operation | Input tensor | Output tensor | Batch size |
|---|---|---|---|
| Query embedding | [1, 256] int64 = 2 KB | [1, 768] f32 = 3 KB | 1 |
| Batch embedding (index) | [32, 256] int64 = 64 KB | [32, 768] f32 = 96 KB | 32 |
| NLI contradiction | [2, 512] int64 = 8 KB | [3] f32 = 12 bytes | 1 pair |

### SQLite configuration

| Setting | Value | Effect |
|---|---|---|
| journal_mode | WAL | Concurrent reads during writes |
| cache_size | -2000 (2 MB) | In-memory page cache |
| page_size | 4096 | OS page-aligned |
| Connection model | Single Mutex<Connection> | No pool, serialized access |

---

## 4. Context Pack Limits (Portable)

### Configured budgets

| Mode | Token budget | Max results | Max hops | Sections observed |
|---|---|---|---|---|
| cold_start | 4096 | 5 rules | 0 | 10 |
| task | 8192 | 25 | 2 | 50 |
| escalation | 8192 | 20 | 3 | 40 |

### Actual measured output

| Mode | Query | Sections | Token estimate | Output chars | Dropped |
|---|---|---|---|---|---|
| cold_start | (none) | 10 | 2,047 | 9,153 | 0 |
| task | "search" | 50 | 5,410 | ~25 KB | 6 |
| task | "how does the search pipeline work" | 50 | 7,120 | 33,377 | 6 |
| task | "fix the search pipeline RRF fusion ranking..." | 50 | 5,199 | ~24 KB | 6 |
| task | "LeftmostLongest Aho-Corasick match kind auto-link" | 50 | 5,770 | ~27 KB | 6 |
| escalation | "search pipeline" | 40 | 6,045 | 27,937 | 0 |
| task (max-tokens=65536) | "search pipeline" | 50 | 6,839 | 31,953 | 0 |

### Upper range

The **section count is capped by search results**, not the token budget.
Even with `--max-tokens 65536`, task mode returns 50 sections because
that's all the search produces (25 direct results + 25 graph-expanded).
The token budget only affects how many of those sections fit before
truncation.

**Maximum context pack size:** ~33 KB (≈7,120 tokens) for task mode
with a broad natural-language query. This is well under the 8192 token
budget. The budget acts as a safety cap, not a binding constraint at
current entity counts.

**Scaling projection:** As the knowledge base grows, more sections will
be produced, and the token budget will become the binding constraint.
At that point, sections are truncated by priority (rules > observations
> knowledge > code) and remaining sections are listed in
`dropped_sources`.

### Hook output sizes (injected into agent context)

| Hook | Output (JSON payload) | Additional context |
|---|---|---|
| session_start | 9.4 KB | Cold-start pack (identity, rules, code map, knowledge index) |
| prompt_submit | 33.7 KB | Task pack (search results + graph expansion) |
| file_save | 0.1 KB | Event confirmation only |
| session_end | 0.1 KB | Consolidation summary |
| stop | 0.1 KB | Event confirmation only |

---

## 5. Summary: Cost of Running CogZ

### Steady-state MCP server (persistent process)

| Scenario | RAM | Disk | CPU |
|---|---|---|---|
| Idle, no models loaded | ~11 MB | 0 | ~0% |
| Active, models loaded | ~820 MB | 0 | ~0% (between calls) |
| During search call | ~820 MB | 0 | spike (ONNX inference) |
| After 5 min idle | ~11 MB | 0 | ~0% (models unloaded) |

### Per-operation cost

| Operation category | RAM spike | Disk I/O | Network |
|---|---|---|---|
| Read-only (status, query, doctor) | 7-13 MB | 0 | 0 |
| FTS search (hooks) | 10-18 MB | 0 | 0 |
| Hybrid search (CLI/MCP) | 242-820 MB | 0 | 0 |
| Write (record/create/update) | 769-820 MB | 1 file write | 0 |
| Index (full rebuild) | 973 MB foreground + 679 MB background | 1276 file reads + DB writes | 0 |
| Self-update | 11 MB | 1 binary replace | 1-2 HTTP requests |

### What a user can expect

- **First install:** 515 MB disk (binary + runtime + models + DB)
- **Per-repo DB:** ~15 MB for a ~1300-entity repo, scales linearly
- **MCP server at idle:** ~11 MB RAM, near-zero CPU
- **MCP server active:** ~820 MB RAM (both embedding models loaded)
- **Hook calls:** <20 MB RAM, sub-second, no models loaded
- **CLI search:** 242-595 MB RAM (one model), <1s
- **Full index:** ~1 GB peak RAM (both models), ~12s foreground + ~2 min background
- **Context packs:** 9-34 KB output, 2,000-7,100 tokens, always under budget
- **Model idle unload:** after 5 min, RAM drops back to ~11 MB

### What is portable vs machine-specific

| Metric | Portable? | Notes |
|---|---|---|
| Binary size | Yes | Fixed bytes |
| DB size | Yes | Fixed bytes for given entity count |
| Model file sizes | Yes | Fixed bytes |
| Embedding storage | Yes | dimension × 4 bytes per entity |
| Tensor sizes | Yes | Fixed by config (batch, seq_len, dim) |
| SQL query counts | Yes | Determined by code |
| Token estimates | Yes | chars/4 heuristic |
| Context pack sizes | Yes | Determined by config + entity count |
| RSS | Reference only | Includes OS/allocator/shared-lib overhead |
| CPU time | No | Machine-dependent |
| Wall time | No | Machine-dependent |

The RAM values are reference measurements. To estimate RAM on another
machine: fixed components (model weights in memory ≈ 1.7-3.7× disk
size, embedding vectors = count × 3 KB, SQLite cache = 2 MB) are
portable. Add platform-specific overhead for ONNX Runtime arena,
OS page cache, and allocator fragmentation (~50-100 MB typical).
