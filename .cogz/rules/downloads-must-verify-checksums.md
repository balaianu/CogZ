---
id: b0d2e3f4-09cc-414e-831c-8f5710374d4a
title: Downloads must verify checksums or fail explicitly
type: rule
status: active
created_at: "2026-09-03T12:12:00Z"
updated_at: "2026-09-03T12:12:00Z"
references: []
category: security
tags: ["security", "checksum", "download", "update"]
confidence: 1.0
---

Any binary download (self-update, ONNX Runtime) must either verify
a checksum or explicitly fail. Silently skipping verification when
a SHA256SUMS file is downloaded but lacks a matching entry is a
security hole.

**Rule:** If a SHA256SUMS file is available but doesn't contain an
entry for the target asset, fail with an error. Do not proceed
with an unverified binary.

**Exception:** If the SHA256SUMS file itself is unavailable (network
error, 404), warn and proceed — this is degraded mode, not a
security bypass. The binary is still downloaded over HTTPS.

**Why:** A compromised release could ship a SHA256SUMS file without
the expected asset entry, causing the binary to be installed without
integrity verification. Failing on missing entries closes this gap.
