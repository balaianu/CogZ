---
id: b3c4d5e6-f789-4abc-def0-123456789002
type: knowledge
status: active
title: "Multi-model embedding and batched ONNX inference"
category: architecture
tags: [embedding, onnx, batched-inference, multi-model, phase-8]
created_at: 2026-08-29T23:20:00Z
updated_at: 2026-08-29T23:20:00Z
references: []
---

## Multi-model configuration

Phase 8 introduced separate embedding models for code and knowledge:

- `[embedding].code_model` — defaults to `nomic-ai/CodeRankEmbed-int8`
- `[embedding].knowledge_model` — defaults to `BAAI/bge-base-en-v1.5`

`OnnxEmbeddingModel::with_model_id()` wires config to actual model
directory paths. When `model_id` is non-empty, the model directory is
`models_base/{model_id}` instead of the hardcoded default.

**Critical bug found and fixed during audit:** `run_status` in
`main.rs` and `embed_query` in `cli.rs` were using
`OnnxEmbeddingModel::new()` (default paths) instead of
`with_model_id()` (configured paths). This meant `cogz status` would
report wrong model availability and `cogz search` would embed queries
with the wrong model if custom models were configured. Both now use
`with_model_id()`.

## Batched ONNX inference

`OnnxEmbeddingModel::embed_batch()` pads and collates input_ids into
a single `[batch, max_seq_len]` tensor, runs one `session.run()` call
per batch, then extracts per-sequence embeddings using the attention
mask. This is true batched inference, not a loop over single inputs.

The batch size is configurable. Padding uses `repeat_n(0i64, ...)`.

## Embedding sync pattern

The `embed_synced` function in `cli.rs` follows a three-phase lock
pattern to avoid holding the DB mutex during inference:

1. **Fetch** entity data under the lock, then drop the lock
2. **Infer** without the lock (potentially slow ONNX inference)
3. **Store** vectors under the lock

Entities are partitioned into code and knowledge sets, each embedded
with the appropriate model. Graceful degradation: if a model is
unavailable, those entities are skipped (FTS-only search still works).
