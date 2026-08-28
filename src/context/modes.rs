//! Context assembly modes — cold_start, task, escalation.
//!
//! Each mode defines different retrieval parameters and section
//! priorities. The mode determines how many results to fetch, how
//! many graph hops to expand, and which entities to include without
//! a query (cold_start).

use serde::{Deserialize, Serialize};

/// How a context pack is assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    /// Session start. Compact pack: repo identity, recent rules,
    /// recent observations. No query needed.
    ColdStart,
    /// Per-prompt. Ranked retrieval using the agent's query, with
    /// graph expansion from matched entities.
    #[default]
    Task,
    /// When a task pack was insufficient. Wider retrieval: larger k,
    /// more expansion hops, lower relevance threshold.
    Escalation,
}

impl ContextMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ColdStart => "cold_start",
            Self::Task => "task",
            Self::Escalation => "escalation",
        }
    }

    /// Parse a mode from a string (case-insensitive). Accepts
    /// "cold_start", "task", "escalation".
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "cold_start" | "coldstart" => Some(Self::ColdStart),
            "task" => Some(Self::Task),
            "escalation" => Some(Self::Escalation),
            _ => None,
        }
    }

    /// Whether this mode requires a query.
    pub fn requires_query(&self) -> bool {
        matches!(self, Self::Task | Self::Escalation)
    }
}

impl std::fmt::Display for ContextMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_modes() {
        assert_eq!(
            ContextMode::parse("cold_start"),
            Some(ContextMode::ColdStart)
        );
        assert_eq!(ContextMode::parse("task"), Some(ContextMode::Task));
        assert_eq!(
            ContextMode::parse("escalation"),
            Some(ContextMode::Escalation)
        );
        assert_eq!(ContextMode::parse("invalid"), None);
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(
            ContextMode::parse("Cold_Start"),
            Some(ContextMode::ColdStart)
        );
        assert_eq!(ContextMode::parse("TASK"), Some(ContextMode::Task));
    }

    #[test]
    fn requires_query() {
        assert!(!ContextMode::ColdStart.requires_query());
        assert!(ContextMode::Task.requires_query());
        assert!(ContextMode::Escalation.requires_query());
    }

    #[test]
    fn serde_roundtrip() {
        let json = serde_json::to_string(&ContextMode::Escalation).unwrap();
        assert_eq!(json, "\"escalation\"");
        let mode: ContextMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, ContextMode::Escalation);
    }
}
