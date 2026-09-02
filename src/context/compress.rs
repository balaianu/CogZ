//! Token budget management — estimation, section prioritization,
//! and truncation.
//!
//! Uses the chars/4 heuristic per the architecture doc. The function
//! is behind a trait so it can be swapped for a real tokenizer later
//! without changing the rest of the system.

use super::ContextSection;
use super::modes::ContextMode;

/// Estimate token count using the chars/4 heuristic.
/// Overestimates for code (safe — packs come in under budget).
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / 4
}

/// Estimate the token cost of a single section (title + content).
pub fn section_tokens(section: &ContextSection) -> usize {
    estimate_tokens(&section.title) + estimate_tokens(&section.content)
}

/// Priority rank for a section type. Lower = higher priority.
/// Identity is highest (it's compact and sets context), then rules
/// (validated conventions), then observations (raw findings), then
/// knowledge and code (context), then observations (supporting detail).
fn source_priority(source: &str) -> u8 {
    match source {
        "identity" => 0,
        "rule" => 1,
        "knowledge" => 2,
        // Code entities from direct search or graph expansion
        "function" | "class" | "file" | "module" => 3,
        "observation" => 4,
        // Derived sections (code map, knowledge index)
        _ => 5,
    }
}

/// Sort sections for token budget fitting.
///
/// For cold_start: sort by type priority (identity, rules, knowledge,
/// code, observations), then by composite score. This ensures the
/// compact structural sections come first.
///
/// For task/escalation: sort by relevance (search score) descending,
/// with type priority as a tiebreaker. The search already ranked
/// results by relevance — respecting that ranking means code entities
/// that matched the query get budget ahead of less-relevant rules.
pub fn sort_by_priority(sections: &mut [ContextSection], mode: ContextMode) {
    match mode {
        ContextMode::Task | ContextMode::Escalation => {
            sections.sort_by(|a, b| {
                b.relevance
                    .partial_cmp(&a.relevance)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| source_priority(&a.source).cmp(&source_priority(&b.source)))
            });
        }
        ContextMode::ColdStart => {
            sections.sort_by(|a, b| {
                source_priority(&a.source)
                    .cmp(&source_priority(&b.source))
                    .then_with(|| {
                        b.relevance
                            .partial_cmp(&a.relevance)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
        }
    }
}

/// Fit sections into a token budget. Returns the sections that fit
/// (possibly with the last one truncated) and a list of dropped
/// section descriptions.
///
/// Sections must already be sorted by priority (see
/// [`sort_by_priority`]). Sections are added in order until the
/// budget is exhausted. The last section that doesn't fully fit is
/// truncated to the remaining budget. All subsequent sections are
/// dropped.
pub fn fit_budget(
    sections: Vec<ContextSection>,
    token_budget: usize,
) -> (Vec<ContextSection>, Vec<String>) {
    let mut kept = Vec::with_capacity(sections.len());
    let mut dropped = Vec::new();
    let mut used = 0usize;

    for section in sections {
        let cost = section_tokens(&section);
        if used + cost <= token_budget {
            used += cost;
            kept.push(section);
        } else if used < token_budget {
            // Truncate this section to fit the remaining budget.
            // Account for the title's token cost when truncating content.
            let remaining = token_budget - used;
            let title_cost = estimate_tokens(&section.title);
            let content_budget = remaining.saturating_sub(title_cost);
            let mut truncated = section;
            truncated.content = truncate_to_tokens(&truncated.content, content_budget);
            kept.push(truncated);
            used = token_budget;
        } else {
            dropped.push(format!(
                "{}:{} (over token budget)",
                section.source, section.title
            ));
        }
    }

    (kept, dropped)
}

/// Truncate content to approximately `max_tokens` tokens using the
/// chars/4 heuristic. Adds an ellipsis if truncated.
fn truncate_to_tokens(content: &str, max_tokens: usize) -> String {
    if max_tokens == 0 {
        return String::new();
    }
    let max_chars = max_tokens * 4;
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let truncated: String = content.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(source: &str, title: &str, content: &str, relevance: f32) -> ContextSection {
        ContextSection {
            source: source.to_string(),
            entity_id: "test-id".to_string(),
            title: title.to_string(),
            content: content.to_string(),
            relevance,
            graph_path: vec!["test-id".to_string()],
            graph_path_description: String::new(),
        }
    }

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens("hello world"), 2); // 11 chars / 4 = 2
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("ab"), 0); // 2 chars / 4 = 0
    }

    #[test]
    fn section_tokens_includes_title() {
        let s = section("rule", "title here", "content here", 0.5);
        // "title here" = 10 chars = 2 tokens
        // "content here" = 12 chars = 3 tokens
        assert_eq!(section_tokens(&s), 5);
    }

    #[test]
    fn cold_start_sort_prioritizes_by_type_then_score() {
        let mut sections = vec![
            section("knowledge", "K", "c", 0.9),
            section("observation", "O", "c", 0.5),
            section("rule", "R", "c", 0.3),
            section("function", "F", "c", 0.8),
        ];
        sort_by_priority(&mut sections, ContextMode::ColdStart);
        assert_eq!(sections[0].source, "rule");
        assert_eq!(sections[1].source, "knowledge");
        assert_eq!(sections[2].source, "function");
        assert_eq!(sections[3].source, "observation");
    }

    #[test]
    fn task_mode_sort_prioritizes_by_relevance() {
        let mut sections = vec![
            section("rule", "R", "c", 0.3),
            section("function", "F", "c", 0.8),
            section("knowledge", "K", "c", 0.5),
        ];
        sort_by_priority(&mut sections, ContextMode::Task);
        // Highest relevance first, regardless of type
        assert_eq!(sections[0].source, "function");
        assert_eq!(sections[1].source, "knowledge");
        assert_eq!(sections[2].source, "rule");
    }

    #[test]
    fn sort_breaks_ties_by_relevance() {
        let mut sections = vec![
            section("rule", "R1", "c", 0.3),
            section("rule", "R2", "c", 0.8),
        ];
        sort_by_priority(&mut sections, ContextMode::ColdStart);
        assert_eq!(sections[0].title, "R2"); // higher relevance first
    }

    #[test]
    fn fit_budget_keeps_all_when_under() {
        let sections = vec![
            section("rule", "R", "short", 0.5),
            section("observation", "O", "short", 0.5),
        ];
        let (kept, dropped) = fit_budget(sections, 100);
        assert_eq!(kept.len(), 2);
        assert!(dropped.is_empty());
    }

    #[test]
    fn fit_budget_drops_when_over() {
        let sections = vec![
            section("rule", "R", "content that is long enough to matter", 0.5),
            section("observation", "O", "more content that is also long", 0.5),
        ];
        let (kept, dropped) = fit_budget(sections, 5);
        assert!(kept.len() <= 2);
        assert!(!dropped.is_empty() || kept.len() < 2);
    }

    #[test]
    fn fit_budget_truncates_last_fitting_section() {
        // Section 1: title(8 chars=2) + content(8 chars=2) = 4 tokens
        // Section 2: title(8 chars=2) + content(8 chars=2) = 4 tokens
        // Section 3: title(8 chars=2) + content(40 chars=10) = 12 tokens
        // Budget: 15 → sections 1+2 fit (8 tokens), section 3 truncated to 7 tokens
        // (remaining=7, title=2, content_budget=5 → 20 chars + "...")
        let sections = vec![
            section("rule", "title123", "content1", 0.5),
            section("rule", "title456", "content2", 0.5),
            section(
                "rule",
                "title789",
                "abcdefghijklmnopqrstuvwxyz0123456789abcd",
                0.5,
            ),
        ];
        let (kept, dropped) = fit_budget(sections, 15);
        assert_eq!(kept.len(), 3);
        assert!(dropped.is_empty());
        assert!(kept[2].content.ends_with("..."));
    }

    #[test]
    fn truncate_adds_ellipsis() {
        let result = truncate_to_tokens("this is a long string", 2);
        assert!(result.ends_with("..."));
        assert!(result.len() < "this is a long string".len());
    }

    #[test]
    fn truncate_preserves_short_content() {
        let result = truncate_to_tokens("short", 10);
        assert_eq!(result, "short");
    }

    #[test]
    fn truncate_zero_budget() {
        let result = truncate_to_tokens("anything", 0);
        assert_eq!(result, "");
    }
}
