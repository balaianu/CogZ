---
id: f2a6b7c8-09cc-414e-831c-8f5710374d4a
title: Temp files must include PID for concurrent safety
type: knowledge
status: active
created_at: "2026-09-03T12:00:00Z"
updated_at: "2026-09-03T12:00:00Z"
references: []
category: gotchas
tags: ["temp-files", "concurrency", "update", "embed-bg"]
---

# Temp files must include PID for concurrent safety

Temp file names derived from version numbers or fixed strings
(e.g. `cogz-update-{version}`) collide when two processes run
concurrently. Two `cogz update` invocations would write to the
same temp file, corrupting each other's downloads.

## The pattern

Always include `std::process::id()` in temp file names:

```rust
let pid = std::process::id();
let temp_binary = temp_dir.join(format!("cogz-update-{version}-{pid}"));
```

## Where this applies in CogZ

1. **Self-update** (`src/update.rs`): temp binary and checksum files
   include PID. Previously used only the version number.

2. **Background embedding** (`src/commands/embed_bg.rs`): ID files
   already include PID (was correct from the start). Added a startup
   sweep that removes stale `cogz-embed-bg-*.txt` files older than
   1 hour — these accumulate when the child process crashes before
   cleaning up.

3. **ONNX Runtime download** (`src/embed/runtime.rs`): temp
   directory includes PID (was correct from the start).

## The 1-hour sweep threshold

The background embed cleanup removes files older than 1 hour. This
preserves ID files from processes that are still running (a large
codebase embedding job can take minutes) while cleaning up files
from crashed processes. The threshold is conservative — if a
background embed takes more than 1 hour, something is wrong anyway.
