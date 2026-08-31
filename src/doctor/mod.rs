//! Health check and retention module.
//!
//! `cogz doctor` reports DB integrity, model availability, file sync
//! consistency, and policy violations. `cogz doctor --prune-observations`
//! handles retention with tombstones.

pub mod checks;
pub mod prune;

pub use checks::{DoctorReport, Issue, IssueKind, run_doctor};
pub use prune::{PruneCandidate, PruneReport, find_prune_candidates, run_prune};
