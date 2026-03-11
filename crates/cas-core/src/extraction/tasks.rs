//! Task knowledge extraction
//!
//! Extracts learnings from task close_reason and structured task notes.
//! This captures valuable knowledge that would otherwise be buried in closed tasks.

use cas_types::{Entry, EntryType, Task};

/// Result of extracting knowledge from a task
#[derive(Debug, Clone, Default)]
pub struct TaskExtractionResult {
    /// Learnings extracted from close_reason
    pub learnings: Vec<ExtractedLearning>,
    /// Discoveries from task notes
    pub discoveries: Vec<ExtractedLearning>,
    /// Decisions from task notes
    pub decisions: Vec<ExtractedDecision>,
    /// Suggested rules from patterns
    pub suggested_rules: Vec<SuggestedRule>,
}

/// A learning extracted from a task
#[derive(Debug, Clone)]
pub struct ExtractedLearning {
    pub content: String,
    pub importance: f32,
    pub tags: Vec<String>,
    pub source_task_id: String,
}

/// A decision extracted from task notes
#[derive(Debug, Clone)]
pub struct ExtractedDecision {
    pub decision: String,
    pub rationale: Option<String>,
    pub alternatives_rejected: Vec<String>,
    pub source_task_id: String,
}

/// A suggested rule from task patterns
#[derive(Debug, Clone)]
pub struct SuggestedRule {
    pub content: String,
    pub paths: Option<String>,
    pub confidence: f32,
    pub source_task_id: String,
}

/// Extract knowledge from task close_reason and notes
pub fn extract_from_task(task: &Task) -> TaskExtractionResult {
    let mut result = TaskExtractionResult::default();

    // Extract from close_reason
    if let Some(ref reason) = task.close_reason {
        result
            .learnings
            .extend(extract_from_close_reason(reason, &task.id));
    }

    // Extract from structured notes (notes is a String, not Option)
    if !task.notes.is_empty() {
        let (discoveries, decisions) = extract_from_notes(&task.notes, &task.id);
        result.discoveries = discoveries;
        result.decisions = decisions;
    }

    result
}

/// Extract learnings from a task's close_reason
/// Parses numbered lists and converts each to a standalone learning
fn extract_from_close_reason(reason: &str, task_id: &str) -> Vec<ExtractedLearning> {
    let mut learnings = Vec::new();

    // Split by numbered items (1., 2., etc.) or bullet points
    let lines: Vec<&str> = reason.lines().collect();
    let mut current_item = String::new();

    for line in lines {
        let trimmed = line.trim();

        // Skip header lines (ending with :)
        if trimmed.ends_with(':') && !trimmed.starts_with('-') && !trimmed.starts_with('*') {
            continue;
        }

        // Check if this starts a new numbered item
        let is_numbered = trimmed
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
            && trimmed.contains('.');
        let is_bullet = trimmed.starts_with('-') || trimmed.starts_with('*');

        if (is_numbered || is_bullet) && !current_item.is_empty() {
            // Save previous item
            if let Some(learning) = parse_learning_item(&current_item, task_id) {
                learnings.push(learning);
            }
            current_item = trimmed.to_string();
        } else if is_numbered || is_bullet {
            current_item = trimmed.to_string();
        } else if !trimmed.is_empty() {
            // Continue previous item
            if !current_item.is_empty() {
                current_item.push(' ');
            }
            current_item.push_str(trimmed);
        }
    }

    // Don't forget the last item
    if !current_item.is_empty() {
        if let Some(learning) = parse_learning_item(&current_item, task_id) {
            learnings.push(learning);
        }
    }

    // If no numbered items found, treat the whole reason as a single learning
    if learnings.is_empty() && reason.len() > 20 {
        learnings.push(ExtractedLearning {
            content: reason.trim().to_string(),
            importance: 0.7,
            tags: vec!["task-resolution".to_string()],
            source_task_id: task_id.to_string(),
        });
    }

    learnings
}

/// Parse a single learning item from numbered/bulleted text
fn parse_learning_item(item: &str, task_id: &str) -> Option<ExtractedLearning> {
    // Remove numbering/bullet
    let content = item
        .trim_start_matches(|c: char| {
            c.is_ascii_digit() || c == '.' || c == '-' || c == '*' || c.is_whitespace()
        })
        .trim()
        .to_string();

    // Skip trivial items
    if content.len() < 15 {
        return None;
    }

    // Determine importance based on content indicators
    let importance = if content.to_lowercase().contains("important")
        || content.to_lowercase().contains("critical")
        || content.to_lowercase().contains("always")
        || content.to_lowercase().contains("never")
    {
        0.9
    } else if content.contains("pattern")
        || content.contains("convention")
        || content.contains("should")
    {
        0.8
    } else {
        0.7
    };

    // Extract tags from content
    let mut tags = vec!["task-resolution".to_string()];
    if content.to_lowercase().contains("ssr") || content.to_lowercase().contains("hydrat") {
        tags.push("ssr".to_string());
    }
    if content.to_lowercase().contains("react") || content.to_lowercase().contains("component") {
        tags.push("react".to_string());
    }
    if content.to_lowercase().contains("test") {
        tags.push("testing".to_string());
    }
    if content.to_lowercase().contains("api") || content.to_lowercase().contains("endpoint") {
        tags.push("api".to_string());
    }

    Some(ExtractedLearning {
        content,
        importance,
        tags,
        source_task_id: task_id.to_string(),
    })
}

/// Extract discoveries and decisions from structured task notes
fn extract_from_notes(
    notes: &str,
    task_id: &str,
) -> (Vec<ExtractedLearning>, Vec<ExtractedDecision>) {
    let mut discoveries = Vec::new();
    let mut decisions = Vec::new();

    let lines: Vec<&str> = notes.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Look for DISCOVERY markers
        if trimmed.contains("DISCOVERY") || trimmed.to_lowercase().contains("discovery") {
            // Get the content after the marker
            let content = trimmed
                .replace("DISCOVERY", "")
                .replace("discovery", "")
                .replace(":", "")
                .trim()
                .to_string();

            if content.len() > 10 {
                discoveries.push(ExtractedLearning {
                    content,
                    importance: 0.9, // Discoveries are high importance
                    tags: vec!["discovery".to_string()],
                    source_task_id: task_id.to_string(),
                });
            }
        }

        // Look for [decision] or decision markers
        if trimmed.to_lowercase().contains("[decision]")
            || trimmed.to_lowercase().contains("decided to")
            || trimmed.to_lowercase().contains("decision:")
        {
            let content = trimmed
                .replace("[decision]", "")
                .replace("[Decision]", "")
                .replace("Decision:", "")
                .replace("decision:", "")
                .trim()
                .to_string();

            if content.len() > 10 {
                // Try to find rationale in following lines
                let rationale = if i + 1 < lines.len() {
                    let next = lines[i + 1].trim();
                    if next.to_lowercase().contains("because")
                        || next.to_lowercase().contains("rationale")
                        || next.starts_with("  ")
                    {
                        Some(next.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                };

                decisions.push(ExtractedDecision {
                    decision: content,
                    rationale,
                    alternatives_rejected: vec![],
                    source_task_id: task_id.to_string(),
                });
            }
        }

        // Look for pivot markers
        if trimmed.to_lowercase().contains("[pivot]") || trimmed.to_lowercase().contains("pivoted")
        {
            let content = trimmed
                .replace("[pivot]", "")
                .replace("[Pivot]", "")
                .trim()
                .to_string();

            if content.len() > 10 {
                decisions.push(ExtractedDecision {
                    decision: format!("Pivoted: {content}"),
                    rationale: None,
                    alternatives_rejected: vec![],
                    source_task_id: task_id.to_string(),
                });
            }
        }
    }

    (discoveries, decisions)
}

/// Convert extracted learnings to Entry objects ready for storage
pub fn learnings_to_entries(learnings: &[ExtractedLearning]) -> Vec<Entry> {
    learnings
        .iter()
        .map(|l| Entry {
            content: l.content.clone(),
            entry_type: EntryType::Learning,
            importance: l.importance,
            tags: l.tags.clone(),
            ..Default::default()
        })
        .collect()
}

/// Build AI prompt for enhanced task knowledge extraction
pub fn build_task_extraction_prompt(task: &Task) -> String {
    format!(
        r#"# Extract Knowledge from Task Completion

## Task
**ID:** {}
**Title:** {}
**Type:** {}

## Resolution Summary
{}

## Task Notes
{}

## Instructions

Convert each insight from the resolution into standalone, reusable learnings.

For each learning:
- Rephrase as an actionable insight (not tied to this specific task)
- Include relevant file paths or patterns mentioned
- Tag with appropriate categories
- Set importance: 0.9 for critical patterns, 0.8 for useful conventions, 0.7 for general insights

## Response Format
```json
{{
  "learnings": [
    {{
      "content": "React SSR hydration requires hydrateRoot instead of createRoot",
      "importance": 0.8,
      "tags": ["react", "ssr", "hydration"],
      "should_be_rule": false
    }},
    {{
      "content": "Use useIsClient hook with useSyncExternalStore for client-only rendering in SSR apps",
      "importance": 0.9,
      "tags": ["react", "ssr", "hooks"],
      "should_be_rule": true,
      "rule_paths": "**/*.tsx"
    }}
  ]
}}
```

Only include high-quality, reusable insights. Skip task-specific details that won't help future work.
"#,
        task.id,
        task.title,
        task.task_type,
        task.close_reason.as_deref().unwrap_or("(none)"),
        if task.notes.is_empty() {
            "(none)"
        } else {
            &task.notes
        }
    )
}

#[cfg(test)]
mod tests {
    use crate::extraction::tasks::*;
    use cas_types::{Priority, TaskStatus, TaskType};

    #[test]
    fn test_extract_from_close_reason_numbered() {
        let reason = r#"Implemented SSR hydration fix:
1. Changed createRoot to hydrateRoot for SSR compatibility
2. Added useIsClient hook using useSyncExternalStore
3. Created ClientOnly wrapper component for browser-only code"#;

        let learnings = extract_from_close_reason(reason, "cas-1234");
        assert_eq!(learnings.len(), 3);
        assert!(learnings[0].content.contains("hydrateRoot"));
        assert!(learnings[1].content.contains("useIsClient"));
        assert!(learnings[2].content.contains("ClientOnly"));
    }

    #[test]
    fn test_extract_from_close_reason_bullets() {
        let reason = r#"Fixed authentication:
- Updated JWT validation to check expiry
- Added refresh token rotation"#;

        let learnings = extract_from_close_reason(reason, "cas-5678");
        assert_eq!(learnings.len(), 2);
    }

    #[test]
    fn test_extract_discoveries_from_notes() {
        let notes = r#"[14:30] Started investigation
DISCOVERY: ExClaude Session API returns messages one-by-one via await_response/2
[15:00] Implemented fix
Decision: Use streaming approach instead of batch"#;

        let (discoveries, decisions) = extract_from_notes(notes, "cas-abcd");
        assert_eq!(discoveries.len(), 1);
        assert!(discoveries[0].content.contains("ExClaude"));
        assert_eq!(decisions.len(), 1);
    }

    #[test]
    fn test_skip_trivial_items() {
        let reason = "1. OK\n2. Done\n3. This is a meaningful learning about the architecture";

        let learnings = extract_from_close_reason(reason, "cas-test");
        // Should only keep the meaningful one
        assert_eq!(learnings.len(), 1);
        assert!(learnings[0].content.contains("architecture"));
    }

    #[test]
    fn test_importance_detection() {
        let reason = "1. Always use hydrateRoot for SSR - critical for hydration\n2. Updated styles for dark mode support";

        let learnings = extract_from_close_reason(reason, "cas-test");
        assert_eq!(learnings.len(), 2);
        // First item should have high importance due to "Always" and "critical"
        assert!(learnings[0].importance >= 0.9);
        // Second item should have lower importance
        assert!(learnings[1].importance < 0.9);
    }

    #[test]
    fn test_learnings_to_entries() {
        let learnings = vec![ExtractedLearning {
            content: "Test learning".to_string(),
            importance: 0.8,
            tags: vec!["rust".to_string()],
            source_task_id: "cas-1234".to_string(),
        }];

        let entries = learnings_to_entries(&learnings);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "Test learning");
        assert_eq!(entries[0].entry_type, EntryType::Learning);
        assert_eq!(entries[0].importance, 0.8);
    }

    #[test]
    fn test_build_task_extraction_prompt() {
        let task = Task {
            id: "cas-1234".to_string(),
            title: "Fix SSR hydration".to_string(),
            task_type: TaskType::Bug,
            status: TaskStatus::Closed,
            priority: Priority::HIGH,
            close_reason: Some("Fixed by using hydrateRoot".to_string()),
            notes: "Some notes here".to_string(),
            ..Default::default()
        };

        let prompt = build_task_extraction_prompt(&task);
        assert!(prompt.contains("cas-1234"));
        assert!(prompt.contains("Fix SSR hydration"));
        assert!(prompt.contains("hydrateRoot"));
    }

    #[test]
    fn test_extract_from_task() {
        let task = Task {
            id: "cas-5678".to_string(),
            title: "Test task".to_string(),
            task_type: TaskType::Task,
            status: TaskStatus::Closed,
            priority: Priority::MEDIUM,
            close_reason: Some("1. Fixed the critical bug in component".to_string()),
            notes: "DISCOVERY: Found a new pattern for handling this".to_string(),
            ..Default::default()
        };

        let result = extract_from_task(&task);
        assert!(!result.learnings.is_empty());
        assert!(!result.discoveries.is_empty());
    }
}
