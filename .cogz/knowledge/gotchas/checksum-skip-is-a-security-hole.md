---
id: e1f5a6b7-09cc-414e-831c-8f5710374d4a
title: Checksum skip on missing entry is a security hole
type: knowledge
status: stale
created_at: "2026-09-03T11:59:00Z"
updated_at: "2026-09-05T12:20:02.821294659+00:00"
references: []
category: gotchas
tags: ["security", "checksum", "update", "download", "onnx"]
---

# Checksum skip on missing entry is a security hole

When downloading binaries (self-update, ONNX Runtime), the checksum
verification flow has a subtle failure mode: the SHA256SUMS file is
downloaded successfully, but it contains no entry for the target
asset. The original code silently skipped verification in this case.

This is a security hole. A compromised release could ship a
SHA256SUMS file without the expected asset entry, causing the
downloaded binary to be installed without any integrity check.

## The correct behavior

- SHA256SUMS file not available (network error, 404): warn and
  proceed (degraded mode — better than blocking all updates)
- SHA256SUMS downloaded but no matching entry: **fail with an error**
- SHA256SUMS downloaded, entry found, hash mismatch: **fail with an
  error**
- SHA256SUMS downloaded, entry found, hash matches: proceed

## Two affected paths

1. **Self-update** (`src/update.rs`): Added `ChecksumEntryNotFound`
   error variant. Fails the update if the SHA256SUMS file doesn't
   contain an entry for the platform asset.

2. **ONNX Runtime download** (`src/embed/runtime.rs`): Downloads
   SHA256SUMS from the release, verifies the archive hash before
   extraction. Falls back to warn-only if SHA256SUMS is unavailable
   (ONNX Runtime is not a CogZ asset — it's Microsoft's release).

## Why warn-only for ONNX Runtime unavailable

ONNX Runtime is optional (FTS-only mode works without it). Blocking
`cogz index` because Microsoft's SHA256SUMS is temporarily
unavailable would be worse than proceeding without verification.
The self-update path is stricter because a corrupted binary is
always worse than no update.
