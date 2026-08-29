//! Code indexing — tree-sitter parsing, gitignore-aware scanning,
//! and code entity synchronization.
//!
//! Phase 8: extract functions, classes, files, and modules from
//! source code into the entity graph with structural edges.

pub mod code_graph;
pub mod gitignore;
pub mod sync;
pub mod tree_sitter;
