//! Consolidation — dedup, contradiction detection, promotion, merge.
//!
//! Every insert triggers lightweight dedup and (when the NLI model is
//! available) contradiction detection. Promotion and merge are
//! background operations triggered by `cogz consolidate` or the
//! `consolidate` MCP tool.

pub mod contradict;
pub mod dedup;
pub mod merge;
pub mod promote;

pub use dedup::{DedupResult, DuplicateWarning, check_duplicate};
