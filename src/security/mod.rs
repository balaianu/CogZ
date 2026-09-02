//! Secret detection — prevents secrets from entering canonical files.
//!
//! Knowledge and rules are committed to git. Observations are gitignored
//! but can still leak through shared directories. This module scans
//! entity content before write and rejects if a secret pattern is found.

pub mod scan;

pub use scan::{ScanResult, scan_content};
