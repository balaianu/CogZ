---
id: a1b2c3d4-rmcp-4dec-ision-000000000001
type: knowledge
status: active
title: rmcp tool_router vis=pub requirement
category: decisions
tags: [mcp, rmcp, macros, modules]
created_at: 2026-08-29T02:40:00Z
updated_at: 2026-08-29T02:40:00Z
---

# rmcp tool_router vis=pub requirement

The `#[tool_router]` macro generates a `tool_router()` method that
returns a `ToolRouter<Self>`. By default this method is private. The
`#[tool_handler]` macro on `impl ServerHandler` generates `call_tool`
which calls `Self::tool_router()`.

## Problem

If `#[tool_router]` is in `tools.rs` and `#[tool_handler]` is in
`server.rs`, the private `tool_router()` method is inaccessible across
modules, causing `E0624: associated function is private`.

## Options considered

1. **`vis = "pub"`** — make `tool_router()` public. Chosen because it's
   the simplest and the function is only used internally by rmcp macros.
2. **`server_handler` flag** — `#[tool_router(server_handler)]` auto-
   generates an empty `impl ServerHandler` with `#[tool_handler]`. But
   this generates an empty impl — you can't add `get_info()` to it.
3. **Keep everything in one file** — would exceed the 400-line limit.

## Decision

Use `#[tool_router(vis = "pub")]` in `tools.rs` and a separate
`#[tool_handler] impl ServerHandler` in `server.rs` with a custom
`get_info()`. The public visibility is harmless — `tool_router()` is
only meaningful within rmcp's macro-generated dispatch code.
