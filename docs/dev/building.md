# Building

This guide covers building CogZ from source, release builds, and cross-compilation.

## Prerequisites

- **Rust 1.88+** (edition 2024)
- **C compiler** — `rusqlite` bundles SQLite and needs a C toolchain (`gcc`/`clang` on Linux, Xcode CLT on macOS, MSVC on Windows)
- **~2 GB free RAM** for a release build (LTO + fat codegen-units)

No ONNX models or GPU are required to build. The binary dynamically loads the ONNX Runtime at first use.

## Debug build

```bash
cargo build
```

Fast compile, slower runtime. Good for development and testing.

## Release build

```bash
cargo build --release
```

The release profile uses:
- `opt-level = 3`
- `lto = "fat"`
- `codegen-units = 1`
- `strip = true`

This produces a stripped, optimized binary at `target/release/cogz`. The build takes ~5–10 minutes on a modern machine due to LTO.

## Install locally

```bash
cargo install --path .
```

Installs to `~/.cargo/bin/cogz`. Useful for testing the binary in the same way the install script delivers it.

## Cross-compilation

The release CI builds for four targets:

| Target | Runner | Asset name |
|---|---|---|
| `x86_64-unknown-linux-gnu` | ubuntu-latest | `cogz-x86_64-unknown-linux-gnu` |
| `aarch64-unknown-linux-gnu` | ubuntu-24.04-arm | `cogz-aarch64-unknown-linux-gnu` |
| `aarch64-apple-darwin` | macos-14 | `cogz-aarch64-apple-darwin` |
| `x86_64-pc-windows-msvc` | windows-latest | `cogz-x86_64-pc-windows-msvc.exe` |

To cross-compile locally:

```bash
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

Cross-compilation may require additional system libraries (linker, C cross-compiler). The CI workflow handles this automatically.

## Release process

Releases are triggered by pushing a tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The `release.yml` workflow:
1. Builds release binaries for all four targets.
2. Strips Unix binaries.
3. Generates SHA256 checksums per asset.
4. Creates a GitHub release with the binaries.
5. Generates a combined `SHA256SUMS` manifest.

The install scripts (`install.sh`, `install.ps1`) download from the latest GitHub release and verify checksums against `SHA256SUMS`.

## ONNX Runtime

The `ort` crate loads the ONNX Runtime dynamically — it is not compiled into the binary. On first use, CogZ:

1. Checks for a system-installed ONNX Runtime.
2. If not found, downloads the appropriate shared library to CogZ's local data directory (`~/.local/share/cogz/` on Linux).
3. Extracts and loads it.

This keeps the binary small (~8 MB) and allows runtime upgrades without recompiling.

## Binary size

| Component | Size |
|---|---|
| Stripped release binary | ~8 MB |
| ONNX Runtime (downloaded on first use) | ~50 MB |
| CodeRankEmbed INT8 model | 139 MB |
| bge-base-en-v1.5 model | 210 MB |
| nli-deberta-v3-xsmall model (quantized) | 87 MB |
| Total with all models | ~550 MB |

## See also

- [Dependencies](dependencies.md) — pinned crate versions
- [Testing](testing.md) — test categories and mock models
