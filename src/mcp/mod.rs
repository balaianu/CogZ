//! MCP server — exposes CogZ tools over the Model Context Protocol.
//!
//! The server holds an `Arc<Storage>` and `Config`. Tool handlers are
//! async methods on `CogzServer`. DB operations go through
//! `tokio::task::spawn_blocking` to avoid blocking the tokio runtime.

pub mod dedup;
pub mod server;
pub mod tools;

pub use server::CogzServer;
