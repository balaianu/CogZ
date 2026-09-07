//! Status state machine — validates all entity status transitions.
//!
//! Legal transitions (from `docs/design/entity-model.md`):
//!
//! ```text
//! active    → stale | rejected | superseded
//! stale     → active
//! rejected  → pruned
//! superseded→ pruned
//! pruned    → (terminal)
//! ```

use super::StorageError;

/// Validate a status transition. Returns `Ok(())` if legal, `Err` if not.
pub fn transition_status(current: &str, next: &str) -> Result<(), StorageError> {
    let allowed: &[&str] = match current {
        "active" => &["stale", "rejected", "superseded"],
        "stale" => &["active"],
        "rejected" => &["pruned"],
        "superseded" => &["pruned"],
        "pruned" => &[],
        _ => {
            return Err(StorageError::IllegalTransition {
                from: current.to_string(),
                to: next.to_string(),
            });
        }
    };

    if !allowed.contains(&next) {
        return Err(StorageError::IllegalTransition {
            from: current.to_string(),
            to: next.to_string(),
        });
    }

    Ok(())
}

/// All valid entity statuses.
pub const VALID_STATUSES: &[&str] = &["active", "stale", "superseded", "rejected", "pruned"];

/// Check if a status string is valid.
pub fn is_valid_status(status: &str) -> bool {
    VALID_STATUSES.contains(&status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_to_stale() {
        assert!(transition_status("active", "stale").is_ok());
    }

    #[test]
    fn active_to_rejected() {
        assert!(transition_status("active", "rejected").is_ok());
    }

    #[test]
    fn active_to_superseded() {
        assert!(transition_status("active", "superseded").is_ok());
    }

    #[test]
    fn stale_to_active() {
        assert!(transition_status("stale", "active").is_ok());
    }

    #[test]
    fn rejected_to_pruned() {
        assert!(transition_status("rejected", "pruned").is_ok());
    }

    #[test]
    fn superseded_to_pruned() {
        assert!(transition_status("superseded", "pruned").is_ok());
    }

    #[test]
    fn rejected_to_active_illegal() {
        assert!(transition_status("rejected", "active").is_err());
    }

    #[test]
    fn superseded_to_active_illegal() {
        assert!(transition_status("superseded", "active").is_err());
    }

    #[test]
    fn pruned_to_anything_illegal() {
        for target in &["active", "stale", "rejected", "superseded", "pruned"] {
            assert!(transition_status("pruned", target).is_err());
        }
    }

    #[test]
    fn active_to_active_illegal() {
        assert!(transition_status("active", "active").is_err());
    }

    #[test]
    fn unknown_status_errors() {
        assert!(transition_status("unknown", "active").is_err());
    }

    #[test]
    fn is_valid_status_checks() {
        assert!(is_valid_status("active"));
        assert!(is_valid_status("pruned"));
        assert!(!is_valid_status("unknown"));
        assert!(!is_valid_status(""));
    }
}
