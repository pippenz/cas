//! Rule extraction from entries
//!
//! Suggests and extracts rules based on entry patterns.

use chrono::{Duration, Utc};
use std::collections::HashMap;

use crate::error::MemError;
use crate::store::{RuleStore, Store};
use crate::types::{Entry, Rule, RuleStatus, Scope};

/// A suggested rule with confidence score
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuleSuggestion {
    /// The suggested rule content
    pub content: String,
    /// Entry IDs this suggestion came from
    pub source_ids: Vec<String>,
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Pattern type that triggered this suggestion
    pub pattern: String,
}

/// Rule extractor
pub struct Extractor<'a> {
    entry_store: &'a dyn Store,
    rule_store: &'a dyn RuleStore,
}

impl<'a> Extractor<'a> {
    /// Create a new extractor
    pub fn new(entry_store: &'a dyn Store, rule_store: &'a dyn RuleStore) -> Self {
        Self {
            entry_store,
            rule_store,
        }
    }

    /// Suggest rules based on entry patterns
    ///
    /// Looks for two patterns:
    /// 1. High-value entries (helpful > harmful)
    /// 2. Repeated themes from keyword frequency
    pub fn suggest(&self, limit: usize) -> Result<Vec<RuleSuggestion>, MemError> {
        let entries = self.entry_store.list()?;
        let mut suggestions = Vec::new();

        // Pattern 1: High-value entries
        let high_value: Vec<_> = entries.iter().filter(|e| e.feedback_score() > 0).collect();

        for entry in high_value.iter().take(limit) {
            let confidence = (entry.feedback_score() as f64 / 10.0).min(1.0);
            suggestions.push(RuleSuggestion {
                content: entry.content.clone(),
                source_ids: vec![entry.id.clone()],
                confidence,
                pattern: "high-feedback".to_string(),
            });
        }

        // Pattern 2: Repeated themes
        let keywords = self.extract_keywords(&entries);
        let repeated = self.find_repeated_themes(&entries, &keywords);

        for theme in repeated
            .into_iter()
            .take(limit.saturating_sub(suggestions.len()))
        {
            suggestions.push(theme);
        }

        // Sort by confidence
        suggestions.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        suggestions.truncate(limit);
        Ok(suggestions)
    }

    /// Extract rule from specific entries
    pub fn extract(
        &self,
        content: &str,
        source_ids: Vec<String>,
        tags: Vec<String>,
        paths: &str,
    ) -> Result<Rule, MemError> {
        self.extract_with_hook(content, source_ids, tags, paths, None)
    }

    /// Extract rule from specific entries with optional hook command
    pub fn extract_with_hook(
        &self,
        content: &str,
        source_ids: Vec<String>,
        tags: Vec<String>,
        paths: &str,
        hook_command: Option<String>,
    ) -> Result<Rule, MemError> {
        let id = self.rule_store.generate_id()?;

        let rule = Rule {
            id,
            scope: Scope::default(),
            created: Utc::now(),
            source_ids,
            content: content.to_string(),
            status: RuleStatus::Draft,
            helpful_count: 0,
            harmful_count: 0,
            tags,
            paths: paths.to_string(),
            last_accessed: None,
            review_after: Some(Utc::now() + Duration::days(30)),
            hook_command,
            category: crate::types::RuleCategory::default(),
            priority: 2,
            surface_count: 0,
            auto_approve_tools: None,
            auto_approve_paths: None,
            team_id: None,
            share: None,
        };

        self.rule_store.add(&rule)?;
        Ok(rule)
    }

    /// Extract keywords from entries
    fn extract_keywords(&self, entries: &[Entry]) -> HashMap<String, usize> {
        let mut freq = HashMap::new();

        // Common stop words to ignore
        let stop_words: std::collections::HashSet<&str> = [
            "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has",
            "had", "do", "does", "did", "will", "would", "could", "should", "may", "might", "must",
            "shall", "can", "need", "dare", "ought", "used", "to", "of", "in", "for", "on", "with",
            "at", "by", "from", "as", "into", "through", "during", "before", "after", "above",
            "below", "between", "under", "again", "further", "then", "once", "here", "there",
            "when", "where", "why", "how", "all", "each", "few", "more", "most", "other", "some",
            "such", "no", "nor", "not", "only", "own", "same", "so", "than", "too", "very", "just",
            "and", "but", "if", "or", "because", "until", "while", "this", "that", "these",
            "those", "it", "its",
        ]
        .into_iter()
        .collect();

        for entry in entries {
            for word in entry.content.split_whitespace() {
                // Clean the word
                let word = word
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase();

                // Skip short words and stop words
                if word.len() > 3 && !stop_words.contains(word.as_str()) {
                    *freq.entry(word).or_insert(0) += 1;
                }
            }
        }

        freq
    }

    /// Find repeated themes based on keyword frequency
    fn find_repeated_themes(
        &self,
        entries: &[Entry],
        keywords: &HashMap<String, usize>,
    ) -> Vec<RuleSuggestion> {
        let threshold = 3;

        // Find common keywords (appear in multiple entries)
        let common: Vec<_> = keywords
            .iter()
            .filter(|entry| *entry.1 >= threshold)
            .map(|(word, _)| word.clone())
            .collect();

        let mut themes = Vec::new();

        for keyword in common.iter().take(5) {
            let matching: Vec<_> = entries
                .iter()
                .filter(|e| e.content.to_lowercase().contains(keyword))
                .collect();

            if matching.len() >= 2 {
                let confidence = (matching.len() as f64 / entries.len().max(1) as f64).min(1.0);

                themes.push(RuleSuggestion {
                    content: format!(
                        "Common pattern related to '{}' found across {} entries",
                        keyword,
                        matching.len()
                    ),
                    source_ids: matching.iter().map(|e| e.id.clone()).collect(),
                    confidence,
                    pattern: "repeated".to_string(),
                });
            }
        }

        themes
    }
}

/// Get high-value entries suitable for rule generation
pub fn get_high_value_entries(store: &dyn Store, limit: usize) -> Result<Vec<Entry>, MemError> {
    let mut entries = store.list()?;

    // Filter to entries with positive feedback
    entries.retain(|e| e.feedback_score() > 0);

    // Sort by feedback score (descending)
    entries.sort_by_key(|e| std::cmp::Reverse(e.feedback_score()));

    entries.truncate(limit);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use crate::rules::*;
    use tempfile::TempDir;

    use crate::store::{SqliteRuleStore, SqliteStore};

    #[test]
    fn test_extract_rule() {
        let temp = TempDir::new().unwrap();
        let entry_store = SqliteStore::open(temp.path()).unwrap();
        entry_store.init().unwrap();

        let rule_store = SqliteRuleStore::open(temp.path()).unwrap();
        rule_store.init().unwrap();

        let extractor = Extractor::new(&entry_store, &rule_store);

        let rule = extractor
            .extract(
                "Always use table-driven tests",
                vec!["2024-01-01-001".to_string()],
                vec!["testing".to_string()],
                "**/*_test.go",
            )
            .unwrap();

        assert!(rule.id.starts_with("rule-"));
        assert_eq!(rule.content, "Always use table-driven tests");
        assert_eq!(rule.paths, "**/*_test.go");
    }

    #[test]
    fn test_get_high_value_entries() {
        let temp = TempDir::new().unwrap();
        let store = SqliteStore::open(temp.path()).unwrap();
        store.init().unwrap();

        let mut entry1 = Entry::new("001".to_string(), "Entry 1".to_string());
        entry1.helpful_count = 5;

        let mut entry2 = Entry::new("002".to_string(), "Entry 2".to_string());
        entry2.helpful_count = 10;

        let entry3 = Entry::new("003".to_string(), "Entry 3".to_string());

        store.add(&entry1).unwrap();
        store.add(&entry2).unwrap();
        store.add(&entry3).unwrap();

        let high_value = get_high_value_entries(&store, 10).unwrap();
        assert_eq!(high_value.len(), 2);
        assert_eq!(high_value[0].id, "002"); // Highest score first
    }
}
