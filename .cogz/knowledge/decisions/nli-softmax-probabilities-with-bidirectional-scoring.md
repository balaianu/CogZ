---
id: f3c86f25-4ef3-4b97-915b-70b3532a362b
title: NLI softmax probabilities with bidirectional scoring
type: knowledge
status: active
created_at: "2026-09-01T12:26:41.940383925+00:00"
updated_at: "2026-09-01T12:26:41.940383925+00:00"
references: []
category: decisions
tags: ["nli", "contradiction", "softmax", "bidirectional"]
---

# NLI Softmax Probabilities with Bidirectional Scoring

The NLI contradiction detection layer in CogZ uses softmax probabilities rather than argmax-only labels. This allows threshold-based contradiction decisions instead of binary classification.

## Configuration

- contradiction_threshold: 0.70 (P(contradiction) must exceed this)
- contradiction_cosine_threshold: 0.85 (pre-filter: skip pairs with low embedding similarity)
- contradiction_length_ratio: 5.0 (pre-filter: skip pairs with very different lengths)

## Bidirectional scoring

Both forward (A to B) and reverse (B to A) NLI classification are run. The maximum P(contradiction) across both directions is used. This catches cases where one direction shows contradiction but the other does not.

## Pre-filters

1. Identical text fast-path: skip NLI for identical texts
2. Length ratio filter: skip if texts differ by more than 5:1
3. Cosine similarity filter: skip if embedding similarity below 0.85

These pre-filters reduce NLI invocations by approximately 90 percent while maintaining detection quality.