---
id: aededb8a-2bee-4336-b25a-3d10e512cba1
title: Context pack section priority in src/context/compress.rs follows a strict hie...
type: observation
status: active
created_at: "2026-09-01T12:34:17.378731187+00:00"
updated_at: "2026-09-01T12:34:17.378731187+00:00"
references: []
source: agent
confidence: 0.5
---

Context pack section priority in src/context/compress.rs follows a strict hierarchy: identity (0) > rule (1) > observation (2) > knowledge (3) > code (4). When the token budget is tight, rules are always included before observations, and observations before knowledge. This means code entities are dropped first when budget is constrained.