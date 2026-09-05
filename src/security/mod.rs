//! Secret detection — prevents secrets from entering canonical files.
//!
//! Knowledge, rules, and observations are all committed to git in
//! team-sharing mode. This module scans entity content before write
//! and rejects if a secret pattern is found.

pub mod scan;

pub use scan::{ScanResult, scan_content};
