---
id: 9f78caad-67ed-48fb-b356-d3684aab214b
title: "MCP tools must require explicit repo parameter, no fallbacks"
type: rule
status: active
created_at: "2026-09-05T07:08:13.895469759+00:00"
updated_at: "2026-09-05T07:08:13.895469759+00:00"
references: ["src/mcp/params.rs", "src/mcp/server.rs"]
confidence: 0.9
---

Every MCP tool must require an explicit `repo` parameter. No fallback to cwd, no implicit session context, no Roots-based discovery. This aligns with the 2026-07-28 MCP spec (SEP-2577) which deprecated Roots in favor of tool parameters, resource URIs, or server configuration. The explicit parameter approach is simpler, less ambiguous, and future-proof — an agent working across multiple repos must state which repo it means.