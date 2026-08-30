# CogZ — Testing Strategy

This document defines how CogZ is tested at every level. The goal is
confidence: every phase of the implementation plan has tests that
prove it works and catch regressions.

---

## Testing Principles

1. **Model-free by default.** Most tests run without ONNX models.
   Models are large, slow to load, and require network access on
   first download. Tests that need embeddings use a mock model.
2. **Filesystem-based fixtures.** Tests create real `.cogz/`
   directories in temp dirs. No mocking the filesystem — file I/O is
   core to the architecture and must be tested for real.
3. **Real SQLite.** No mocking the database. Tests use real SQLite
   (WAL mode, FTS5, vec0) in temp paths. This catches real SQL
   errors, schema issues, and concurrency problems.
4. **Incremental.** Each phase adds tests. Tests from earlier phases
   must still pass. No phase breaks previous tests.
5. **Fast.** The full test suite runs in under 10 seconds (excluding
   model-dependent tests, which are opt-in).

---

## Test Categories

### Unit Tests

Live in each module as `#[cfg(test)]` blocks. Test individual
functions and structs in isolation.

| Module | What's tested | Phase |
|---|---|---|
| `config/` | Config parsing, validation, default generation, project name autodetection | 1 |
| `storage/schema.rs` | Schema creation, migrations, version tracking | 2 |
| `storage/crud.rs` | Insert, update, get, delete entities; edge CRUD | 2 |
| `storage/query.rs` | Type/status/file_path queries; graph traversal (1-hop, 2-hop, N-hop) | 2 |
| `storage/edges.rs` | Edge CRUD, batch edge queries, chunked IN queries | 2 |
| `storage/graph.rs` | Graph traversal, batch neighbors, path queries | 2 |
| `storage/embeddings.rs` | vec0 insert, delete, KNN search | 4 |
| `storage/events.rs` | Event recording, event query | 2 |
| `storage/status.rs` | Status state machine: all legal transitions succeed, all illegal transitions fail (rejected→active, superseded→active, pruned→active) | 2 |
| `files/entities.rs` | Entity file structs, UUID generation, file path derivation | 3 |
| `files/frontmatter.rs` | Custom YAML frontmatter parser: parsing, writing, round-trip | 3 |
| `files/sync.rs` | Change detection (content hash), sync state machine, stale flagging, policy-aware content change handling per entity type | 3 |
| `index/gitignore.rs` | .gitignore parsing, path filtering, allow overrides | 8 |
| `index/tree_sitter.rs` | AST parsing, entity extraction (Rust) | 8 |
| `index/tree_sitter/python.rs` | AST parsing, entity extraction (Python) | 8 |
| `index/code_graph/mod.rs` | Edge construction — calls, imports, extends (Rust) | 8 |
| `index/code_graph/python.rs` | Edge construction — calls, imports, extends (Python) | 8 |
| `index/sync/mod.rs` | UUID v5 generation, content hash, stale marking, entity sync | 8 |
| `index/git_diff.rs` | Diff parsing, changed file detection | 10 (planned) |
| `embed/cache.rs` | Cache hit/miss, content-hash-based lookup | 4 |
| `search/rrf.rs` | RRF fusion correctness, score computation | 5 |
| `search/expand.rs` | Graph expansion, path recording | 5 |
| `search/describe.rs` | Batched path description, provenance strings | 5 |
| `context/compress.rs` | Token budgeting, section prioritization, truncation | 6 |
| `consolidate/dedup.rs` | Similarity threshold, duplicate flagging, exact title match, fuzzy title match | 9 (planned) |
| `consolidate/merge.rs` | Edge redirection, superseded marking, `superseded_by` field set | 9 (planned) |
| `consolidate/promote.rs` | Promotion threshold, rule creation from observation, `derived_from` edge | 9 (planned) |
| `hooks/lifecycle.rs` | Event handling, context pack generation per mode | 11 (planned) |

### Integration Tests

Live in `tests/` directory. Test multiple modules working together.

| Test file | What's tested | Phase |
|---|---|---|
| `tests/test_file_sync.rs` | File → DB sync end-to-end: create file, index, verify DB entity; edit file, reindex, verify update; delete file, reindex, verify stale; observation content edit logs `observation_edited` event | 3 |
| `tests/test_entities.rs` | Entity file structs, UUID generation, file path derivation | 3 |
| `tests/test_frontmatter.rs` | Custom YAML frontmatter parser: parsing, writing, round-trip, edge cases | 3 |
| `tests/test_search.rs` | Full search pipeline: insert entities, run hybrid search, verify ranking and graph expansion | 5 |
| `tests/test_context.rs` | Context assembly: insert entities with references, run each mode, verify pack structure and token budget | 6 |
| `tests/test_embed_sync.rs` | Embedding sync pipeline: embed entities, store embeddings, cache behavior | 4 |
| `tests/test_mcp_server.rs` | MCP protocol: start server, call each implemented tool, verify responses; `update_knowledge` edits knowledge; `create_knowledge` returns `duplicate_warning` when title matches | 7 |
| `tests/test_code_index.rs` | Tree-sitter indexing: index a test repo, verify code entities and structural edges | 8 |
| `tests/test_consolidation.rs` | Consolidation pipeline: insert duplicates, verify dedup + `duplicate_warning`; insert same-title knowledge, verify title match; insert contradictions, verify flagging; insert supporting observations, verify promotion; verify `superseded_by` and `derived_from` edges | 9 (planned) |
| `tests/test_update_policy.rs` | Update policy enforcement: `update_knowledge` edits knowledge in-place; no `update_observation` or `edit_rule` tool exists; status state machine rejects illegal transitions; observation content edit detected by sync, event logged | 7 (planned) |
| `tests/test_git_diff.rs` | Change detection: index, modify source file, reindex, verify stale flagging on referenced knowledge | 10 (planned) |

### End-to-End Tests

Live in `tests/` directory. Test the full system from CLI to output.
Not yet implemented — planned for future phases.

| Test file | What's tested | Phase |
|---|---|---|
| `tests/e2e_init.rs` | `cogz init` creates correct directory structure, config, .gitignore | 1 (planned) |
| `tests/e2e_lifecycle.rs` | Full lifecycle: init → index → search → context → record observation → reindex → search again → consolidate → update knowledge → status | 7 (planned) |
| `tests/e2e_reset.rs` | `cogz reset` drops DB, `cogz reset --purge` removes observations but not knowledge/rules | 2 (planned) |
| `tests/e2e_fresh_clone.rs` | Simulate fresh clone: copy knowledge + rules (no DB, no observations), run `cogz index`, verify full DB rebuild | 7 (planned) |
| `tests/e2e_doctor.rs` | `cogz doctor` on healthy repo reports no issues; edit observation file directly → doctor reports policy violation; create near-duplicate knowledge → doctor reports similarity; delete knowledge file without reindex → doctor reports missing file; `cogz doctor --prune-observations` dry-run reports rejected/superseded observations by age; `--confirm` prunes observations, preserves tombstones, edges to tombstones remain valid; active/stale observations are not pruned | 12 (planned) |

---

## Mock Embedding Model

Tests that need embeddings use a deterministic mock:

```rust
pub struct MockEmbeddingModel;

impl EmbeddingModel for MockEmbeddingModel {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // Deterministic hash-based embedding: same text → same vector
        // Different texts → different vectors (with high probability)
        Ok(texts.iter().map(|t| {
            let mut hasher = DefaultHasher::new();
            t.hash(&mut hasher);
            let h = hasher.finish();
            (0..768).map(|i| ((h >> (i % 64)) & 1) as f32).collect()
        }).collect())
    }

    fn dimension(&self) -> usize { 768 }
    fn model_name(&self) -> &str { "mock" }
}
```

This gives:
- Deterministic results (same input → same output across runs)
- Meaningful similarity (identical texts have identical embeddings)
- Meaningful difference (different texts have different embeddings)
- No network access, no model download, no ONNX dependency
- Fast (hash computation, not neural inference)

Tests that require real model quality (e.g., "does dedup actually
find semantically similar observations?") are marked
`#[ignore]` and run explicitly:

```bash
cargo test -- --ignored  # run model-dependent tests
```

---

## Test Fixtures

Tests create temporary directories with `.cogz/` structure and
inline test data. There are no static fixture files — everything is
generated in `tempfile::tempdir()` at test time. This keeps tests
self-contained and avoids fixture maintenance.

### Test repo (inline)

Code indexing tests create a minimal Rust project inside a temp dir
with `src/main.rs` and `src/lib.rs` containing functions and a struct.
This is small enough to index in milliseconds but complex enough to
produce meaningful entities and edges.

### Entity file fixtures (inline)

Entity file tests create sample observation, rule, and knowledge files
inline in temp dirs. Invalid files (missing required fields, bad
references, illegal status transitions) are also generated inline.

### Config fixture (inline)

Tests define a minimal valid config as a constant string and write it
to a temp dir's `.cogz/config.toml`:

```toml
[project]
name = "test-repo"

[storage]
db_path = ".cogz/cogz.db"

[embedding]
code_model = "mock"
knowledge_model = "mock"
dimension = 768

[search]
fts_weight = 0.4
vec_weight = 0.6
rrf_k = 60
max_results = 20

[consolidation]
dedup_threshold = 0.92
title_match_threshold = 0.85
contradiction_check = false
promotion_threshold = 3

[index]
allow = []

[retention]
observation_prune_after_days = 90
tombstone_max_count = 1000
```

Note: `contradiction_check = false` — NLI model is not available in
tests. Dedup still works (embedding-based, uses mock model).

---

## Test Helpers

### `temp_repo()`

Creates a temp directory with `.cogz/` structure, returns the path.
Used by most integration tests.

```rust
fn temp_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let cogz = dir.path().join(".cogz");
    fs::create_dir_all(cogz.join("knowledge")).unwrap();
    fs::create_dir_all(cogz.join("rules")).unwrap();
    fs::create_dir_all(cogz.join("observations")).unwrap();
    fs::write(cogz.join("config.toml"), TEST_CONFIG).unwrap();
    dir
}
```

### `temp_db()`

Creates a temp SQLite DB with the full schema, returns a connection.

```rust
fn temp_db() -> Connection {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let conn = Connection::open(path).unwrap();
    run_migrations(&conn).unwrap();
    conn
}
```

### `insert_test_entity()`

Inserts an entity with a mock embedding, returns its UUID.

```rust
fn insert_test_entity(conn: &Connection, r#type: &str, content: &str) -> String {
    let id = Uuid::new_v4().to_string();
    let entity = Entity::new(&id, r#type, "Test", content);
    insert_entity(conn, &entity).unwrap();
    let embedding = MockEmbeddingModel.embed(&[content]).unwrap().pop().unwrap();
    insert_embedding(conn, &id, &embedding).unwrap();
    id
}
```

---

## Model-Dependent Tests

Tests that require real ONNX models are:

1. Marked `#[ignore]`
2. Documented with what they verify
3. Run explicitly via `cargo test -- --ignored`
4. Run in CI only when models are cached (environment flag
   `COGZ_TEST_MODELS=1`)

| Test | What it verifies |
|---|---|
| `test_real_embedding_dimension` | CodeRankEmbed produces 768-dim vectors |
| `test_real_dedup_semantic` | Semantically similar observations are flagged as duplicates |
| `test_real_contradiction` | Contradictory rules are detected by NLI model |
| `test_real_search_quality` | Search returns relevant results for natural language queries |

These tests are not part of the default test run. They're for
validating model integration before release.

---

## Test Execution

### Default (fast, no models)

```bash
cargo test
```

Runs all unit + integration + e2e tests with mock models. Completes
in under 10 seconds. This is the developer's inner loop.

### With models (slow, requires download)

```bash
COGZ_TEST_MODELS=1 cargo test -- --ignored
```

Runs model-dependent tests. Requires models to be downloaded first
(`cogz index` in any repo triggers download). Used before release.

### Specific phase

```bash
cargo test test_file_sync     # integration test for file sync
cargo test storage::          # all storage unit tests
cargo test e2e_               # all end-to-end tests
```

### Coverage

```bash
cargo tarpaulin --out Html
```

Target: 80% line coverage for MVP (phases 1-8). Consolidation
(phase 9) may have lower coverage initially due to NLI complexity.

---

## CI Pipeline (Future)

GitHub Actions on push and PR:

1. **Build:** `cargo build --release` on x86_64-unknown-linux-gnu
2. **Test:** `cargo test` (default, no models)
3. **Lint:** `cargo clippy -- -D warnings`
4. **Format:** `cargo fmt --check`
5. **Model tests:** Only on release branch, with `COGZ_TEST_MODELS=1`

CI fails on any warning. Clippy is strict. Formatting is enforced.

---

## What's NOT Tested

- **Network operations:** Model downloads are not tested in CI.
  They're tested manually and in model-dependent tests.
- **Concurrent MCP access:** Multi-client concurrency is tested
  manually (start two MCP servers, call tools simultaneously). Rust's
  ownership model provides compile-time safety; runtime concurrency
  bugs are caught by integration tests with multiple threads.
- **Performance:** No automated benchmarks in the test suite.
  Performance is profiled manually during Phase 12. A `cargo bench`
  setup can be added if performance regressions become a concern.
- **Tree-sitter grammar correctness:** We trust the tree-sitter
  grammars to parse correctly. We test that our extraction logic
  produces the right entities from parsed ASTs, not that the ASTs
  themselves are correct.
