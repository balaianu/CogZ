//! CogZ — Local-first, code-aware engineering cognition runtime.
//!
//! This crate provides the library API used by the CLI and MCP server.
//! The public surface grows phase by phase per the implementation plan.

pub mod config;
pub mod consolidate;
pub mod context;
pub mod doctor;
pub mod embed;
pub mod files;
pub mod hooks;
pub mod index;
pub mod init;
pub mod mcp;
pub mod search;
pub mod security;
pub mod storage;
pub mod update;
