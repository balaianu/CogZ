---
id: 5f63d0c4-fadd-4425-87b2-a4e0fda9b1f5
title: The NLI model in CogZ uses softmax probabilities with bidirectional scoring. ...
type: rule
status: active
created_at: "2026-09-01T12:34:35.613905571+00:00"
updated_at: "2026-09-01T12:34:35.613905571+00:00"
references: []
confidence: 0.7
validation_count: 0
supporting_ids: ["6227e754-866d-48ff-a811-2d115a6cd5ca", "040130b4-1f5c-48c0-a389-5b18850f4fbf", "9921acb2-8b73-4de3-b968-a1bd47170618"]
promoted_from: 4f5ae732-8abc-4d10-b061-299bae266d58
derived_from: 4f5ae732-8abc-4d10-b061-299bae266d58
---

The NLI model in CogZ uses softmax probabilities with bidirectional scoring. The contradiction threshold is 0.70, cosine pre-filter is 0.85, and length ratio filter is 5.0. This was implemented to match the Python CogZ benchmark findings.