---
id: e1f5a6b7-09cc-414e-831c-8f5710374d4a
title: Checksum skip on missing entry is a security hole
type: knowledge
status: active
created_at: "2026-09-03T11:59:00Z"
updated_at: "2026-09-05T20:25:00Z"
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

## Both paths now enforce the rule

1. **Self-update** (`src/update.rs`): `ChecksumEntryNotFound` error
   variant. Fails the update if SHA256SUMS doesn't contain an entry
   for the platform asset.

2. **ONNX Runtime download** (`src/embed/runtime.rs`): Returns an
   error if SHA256SUMS is downloaded but has no entry for the ORT
   asset. Only falls back to warn-only when SHA256SUMS itself is
   unavailable (network error, 404) — that's degraded mode, not a
   security bypass.

## Why warn-only for SHA256SUMS unavailable (not missing entry)

ONNX Runtime is optional (FTS-only mode works without it). Blocking
`cogz index` because Microsoft's SHA256SUMS endpoint is temporarily
unavailable would be worse than proceeding without verification —
the download is still over HTTPS. But if the file IS available and
doesn't mention the asset we're downloading, that's suspicious and
must fail.
