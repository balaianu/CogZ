---
id: b8d9f0a1-2345-6789-abcd-ef1234567890
type: knowledge
category: architecture
title: ONNX Runtime discovery and auto-download
tags: [onnx, runtime, embedding, infrastructure]
created_at: 2026-08-31T19:50:00Z
updated_at: 2026-08-31T19:50:00Z
---

# ONNX Runtime discovery and auto-download

## Overview

CogZ uses ONNX Runtime for model inference via the `ort` crate's
`load-dynamic` feature. The runtime shared library is loaded at
runtime, not linked at build time. This avoids build-time TLS
dependencies and enables graceful degradation.

## Discovery order

1. `ORT_DYLIB_PATH` environment variable
2. `~/.local/share/cogz/lib/libonnxruntime.so` (CogZ-managed)
3. System library paths (e.g.
   `/usr/lib/x86_64-linux-gnu/libonnxruntime.so.1.23`)

If none is found, CogZ downloads ONNX Runtime 1.27.0 (CPU-only, ~23MB)
from GitHub releases to `~/.local/share/cogz/lib/libonnxruntime.so`.

## Why load-dynamic instead of download-binaries

The `download-binaries` feature links ORT at build time but requires
`pkg-config` and OpenSSL dev headers. Many systems don't have these.
`load-dynamic` has no build-time dependencies and the runtime can be
obtained automatically on first use.

## Session configuration

- `GraphOptimizationLevel::Level3` — maximum graph optimization
- `with_memory_pattern(false)` — prevents ORT's arena from growing
  unbounded during batched inference. Without this, RSS climbs to 5GB+
  on large embedding workloads.
- Dynamic input detection: the session inspects `session.inputs()` to
  determine whether `token_type_ids` is declared. CodeRankEmbed does
  not declare it; BERT-based models do. Passing an undeclared input
  causes a runtime error.

## ort crate version

Pinned to `=2.0.0-rc.13` (published 2026-07-28). Supports ORT 1.28
API. Matches FastEmbed's runtime version (1.27.0) for comparable
inference performance.

## Graceful degradation

When no runtime is available and download fails, CogZ falls back to
FTS-only search. Model loading logs a warning and sets `available:
false` in status. No panics.
