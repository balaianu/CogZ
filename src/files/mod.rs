//! File layer — canonical markdown entity files and DB synchronization.
//!
//! Files are the source of truth. The database is a derived index.
//! This module handles reading, writing, and syncing entity files
//! to the storage layer.

pub mod embed_sync;
pub mod entities;
pub mod events;
pub mod frontmatter;
pub mod refs;
pub mod sync;

pub use entities::{EntityFile, FileEntityType, read_entity_file, slugify, write_entity_file};
pub use frontmatter::{FmValue, Frontmatter, FrontmatterError};
pub use sync::{
    SyncResult, content_hash, scan_entity_files, sync_all, sync_incremental, sync_single_file,
};
