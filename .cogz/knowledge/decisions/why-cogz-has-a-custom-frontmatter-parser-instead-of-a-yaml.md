---
id: d494d63a-05f4-41fc-91a8-c56b5e525107
title: Why CogZ has a custom frontmatter parser instead of a YAML crate
type: knowledge
status: active
created_at: 2026-08-28T19:26:00Z
updated_at: 2026-08-28T19:26:00Z
references: []
category: decisions
tags: ["frontmatter", "yaml", "dependencies", "parser"]
---

`src/files/frontmatter.rs` is a hand-rolled YAML subset parser
(339 lines). It handles only: scalar key-value pairs, inline string
arrays, and basic types (string, int, float, bool).

**Why not `serde_yaml` or `yaml-rust2`:**

1. The frontmatter format is intentionally constrained — no nested
   mappings, no anchors, no multi-line strings. A full YAML parser
   is overkill for what is effectively `key: value` pairs.

2. Fewer dependencies. CogZ already has 15+ crates. Each one adds
   build time, binary size, and supply chain surface.

3. The parser preserves key insertion order (via `Vec<(String,
   FmValue)>`), which matters for stable file serialization. Most
   YAML parsers use `HashMap` or `BTreeMap` internally, losing
   order.

**Known limitations:**

- No multi-line strings (block scalars `|` and `>`)
- No nested mappings
- No anchors or aliases
- Inline arrays only (`["a", "b"]`), not block arrays

These are by design. If entity files ever need nested structures,
the `properties` JSON field in the DB schema is the escape hatch —
complex data goes there, not in frontmatter.
