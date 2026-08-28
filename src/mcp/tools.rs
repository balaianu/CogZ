//! MCP tool definitions and handlers.
//!
//! Each tool is a method on `CogzServer` annotated with `#[tool]`.
//! Parameter structs derive `serde::Deserialize` and
//! `schemars::JsonSchema` for automatic schema generation.
