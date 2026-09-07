---
id: 040130b4-1f5c-48c0-a389-5b18850f4fbf
title: "Support: NLI softmax verified in source"
type: observation
status: superseded
created_at: "2026-09-01T12:30:00+00:00"
updated_at: "2026-09-05T11:45:22.487109735+00:00"
references: []
supporting_ids: ["4f5ae732-8abc-4d10-b061-299bae266d58"]
source: agent
confidence: 0.7
superseded_by: d69f5e39-60e9-41a1-b713-af428bc85b4a
---

Verified that NliProbabilities in src/embed/model.rs contains contradiction, entailment, and neutral f32 fields derived from softmax.
