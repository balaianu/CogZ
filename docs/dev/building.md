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

The install scripts (`install.sh`, `install.ps1`) download from the latest GitHub release and verify checksums against `SHA256SUMS`. `install.sh` works on Linux, macOS, and Windows (Git Bash / MSYS2). `install.ps1` is the PowerShell alternative for Windows.

## ONNX Runtime

The `ort` crate loads the ONNX Runtime dynamically — it is not compiled into the binary. On first use, CogZ:

1. Checks for a system-installed ONNX Runtime.
2. If not found, downloads the appropriate shared library to CogZ's local data directory (`~/.local/share/cogz/` on Linux).
3. Extracts and loads it.

This keeps the binary small (~28 MB) and allows runtime upgrades without recompiling.

## Binary size

| Component | Size |
|---|---|
| Stripped release binary | ~28 MB |
| ONNX Runtime (downloaded on first use) | ~24 MB |
| CodeRankEmbed INT8 model | 139 MB |
| bge-base-en-v1.5 model | 210 MB |
| nli-deberta-v3-xsmall model (quantized) | 87 MB |
| Total with all models | ~550 MB |

## Trust boundaries and supply chain

CogZ downloads three categories of external code. Each has a different
trust model and different mitigations.

### Self-update (`cogz update`)

Downloads a new binary from GitHub releases. The SHA-256 checksum is
verified against `SHA256SUMS` from the same release. If the checksum
manifest is missing or lacks an entry for the target asset, the update
fails — it never proceeds with an unverified binary.

**Residual risk:** The checksum and binary live on the same GitHub
release. A compromised GitHub account or release could replace both.
There is no out-of-band signature verification (e.g. Sigstore/cosign).
This is the same trust model as `cargo install` trusting crates.io.
Users who want stronger guarantees should build from source.

### ONNX Runtime auto-download

Downloads the ONNX Runtime shared library from Microsoft's GitHub
releases. The SHA-256 checksum is verified against `SHA256SUMS` from
the same release. If the manifest exists but lacks the asset entry,
the download fails. If the manifest itself is unreachable (404 or
network error), the download proceeds over HTTPS as degraded mode —
Microsoft does not always ship `SHA256SUMS` for every release.

Archive extraction uses `tar --no-absolute-names` to prevent
path-traversal (tar slip). Extracted paths are verified to be inside
the extraction directory before loading.

**Residual risk:** A compromised Microsoft GitHub account could
replace both the library and the checksum. A shared library loaded
into the process is as powerful as the binary itself.

### HuggingFace model download

Downloads ONNX models from HuggingFace via `hf-hub` with
content-addressed caching. HuggingFace's git-LFS storage provides
integrity against transit-level corruption (failed downloads are
detected by hash mismatch), but not against repo compromise.

**Residual risk:** A compromised model repo could serve a malicious
model. CogZ does not maintain a hardcoded hash manifest because
community ONNX exports update independently. Users who want stronger
guarantees can download models manually, verify them, and place them
in the cache directory.

### Summary

| Download | Checksum verified | Out-of-band signature | Trust root |
|---|---|---|---|
| Self-update | Yes (SHA256SUMS, hard fail on missing) | No | GitHub releases |
| ONNX Runtime | Yes (SHA256SUMS, hard fail on missing entry) | No | Microsoft GitHub releases |
| HF models | Content-addressed (hf-hub) | No | HuggingFace Hub |

All downloads use HTTPS. No telemetry, no accounts, no cloud services.

## See also

- [Dependencies](dependencies.md) — pinned crate versions
- [Testing](testing.md) — test categories and mock models
