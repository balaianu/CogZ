# Testing

All tests run offline — no network access or model downloads are needed.

## Running tests

```bash
cargo test               # all tests
cargo test --lib         # library tests only
cargo test --test <name> # a specific integration test
cargo test -- --nocapture  # show println! output
```

## Test categories

### Library unit tests

Module-level `#[cfg(test)] mod tests` blocks. These test individual functions and types in isolation. The majority of tests are here.

### Integration tests

Files in `tests/` test the CLI and MCP server end-to-end by running the binary as a subprocess. Subprocess tests spawn the actual `cogz` binary, send JSON-RPC over stdio, and verify responses. This catches serialization, protocol, and integration bugs that in-process tests miss.

### Mock models

Tests use `MockEmbeddingModel` and `MockNliModel` (in `src/embed/model.rs`) instead of real ONNX models. Mocks return deterministic embeddings and NLI labels without loading the ONNX Runtime.

This means:
- Tests run in milliseconds, not seconds.
- No model downloads or disk space needed.
- No ONNX Runtime dependency for testing.
- Embedding behavior is deterministic and reproducible.

## Test fixtures

Tests create temporary directories with `tempfile::tempdir()` and build minimal `.cogz/` structures in them. No shared state between tests — each test gets its own isolated directory and database.

For tests that need a populated database, helper functions in `tests/common/` create entities, edges, and embeddings directly.

## What to test

When adding a feature:

1. **Unit test the core logic.** Test the function in isolation with mock dependencies.
2. **Integration test the CLI path.** If the feature is exposed via a CLI command, add a subprocess test.
3. **Integration test the MCP path.** If the feature is exposed via an MCP tool, add a subprocess MCP test.
4. **Test edge cases.** Empty inputs, missing files, corrupt data, concurrent access (single-threaded but reentrant code paths).
5. **Test degradation.** If the feature uses models, test the FTS-only fallback.

## Coverage

There is no automated coverage tool configured. Coverage is assessed by:
- Every public function having at least one test.
- Every CLI command having at least one subprocess test.
- Every MCP tool having at least one subprocess test.
- Every error path having a test that triggers it.

## CI

GitHub Actions runs on every push and PR to `master`:

1. `cargo fmt --check`
2. `cargo clippy -- -D warnings`
3. `cargo build --release`
4. `cargo test`

This runs on three platforms: Ubuntu x86_64, macOS arm64, Windows x86_64. All must pass before merge.

## See also

- [Building](building.md) — build commands and release process
- [Conventions](conventions.md) — code patterns
