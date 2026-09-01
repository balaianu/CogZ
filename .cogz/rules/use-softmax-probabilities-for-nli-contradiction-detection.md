---
id: 077cf823-5cc5-4251-b6b0-dd14ea8f6c05
title: Use softmax probabilities for NLI contradiction detection
type: rule
status: active
created_at: "2026-09-01T12:27:21.108864327+00:00"
updated_at: "2026-09-01T12:27:21.108864327+00:00"
references: []
confidence: 0.9
---

NLI contradiction detection must use softmax probabilities with bidirectional scoring, not argmax-only labels. The contradiction threshold is 0.70, with cosine similarity and length ratio pre-filters. This matches the Python CogZ benchmark findings.