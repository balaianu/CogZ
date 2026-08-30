//! Dedup re-export — the implementation now lives in `consolidate/dedup.rs`.
//!
//! This shim preserves the existing `crate::mcp::dedup::check_duplicate`
//! import path used by MCP helpers. New code should import from
//! `crate::consolidate` directly.

pub use crate::consolidate::dedup::{DedupResult, DuplicateWarning, check_duplicate};
