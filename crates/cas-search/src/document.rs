//! SearchDocument implementations for CAS domain types
//!
//! This module provides [`SearchDocument`] implementations for all
//! CAS domain types, enabling them to be indexed and searched.

use std::collections::HashMap;

use cas_code::CodeSymbol;
use cas_types::{Entry, Rule, Skill, Spec, Task};

use crate::traits::SearchDocument;

// =============================================================================
// Entry implementation
// =============================================================================

impl SearchDocument for Entry {
    fn doc_id(&self) -> &str {
        &self.id
    }

    fn doc_content(&self) -> &str {
        &self.content
    }

    fn doc_type(&self) -> &str {
        "entry"
    }

    fn doc_tags(&self) -> Vec<&str> {
        self.tags.iter().map(|s| s.as_str()).collect()
    }

    fn doc_metadata(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("entry_type".into(), self.entry_type.to_string());
        m.insert("scope".into(), self.scope.to_string());
        m.insert("importance".into(), self.importance.to_string());
        m.insert("helpful_count".into(), self.helpful_count.to_string());
        m.insert("harmful_count".into(), self.harmful_count.to_string());
        m.insert("archived".into(), self.archived.to_string());
        m.insert("memory_tier".into(), format!("{:?}", self.memory_tier));
        if let Some(ref title) = self.title {
            m.insert("title".into(), title.clone());
        }
        if let Some(ref session_id) = self.session_id {
            m.insert("session_id".into(), session_id.clone());
        }
        if let Some(ref source_tool) = self.source_tool {
            m.insert("source_tool".into(), source_tool.clone());
        }
        m
    }

    fn doc_title(&self) -> Option<&str> {
        self.title.as_deref()
    }
}

// =============================================================================
// Task implementation
// =============================================================================

impl SearchDocument for Task {
    fn doc_id(&self) -> &str {
        &self.id
    }

    fn doc_content(&self) -> &str {
        // Combine title, description, design, and acceptance criteria for search
        // But since we need to return &str, we return description as primary
        &self.description
    }

    fn doc_type(&self) -> &str {
        "task"
    }

    fn doc_tags(&self) -> Vec<&str> {
        self.labels.iter().map(|s| s.as_str()).collect()
    }

    fn doc_metadata(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("title".into(), self.title.clone());
        m.insert("status".into(), self.status.to_string());
        m.insert("priority".into(), format!("{:?}", self.priority));
        m.insert("task_type".into(), self.task_type.to_string());
        m.insert("scope".into(), self.scope.to_string());
        if let Some(ref assignee) = self.assignee {
            m.insert("assignee".into(), assignee.clone());
        }
        if let Some(ref external_ref) = self.external_ref {
            m.insert("external_ref".into(), external_ref.clone());
        }
        if let Some(ref close_reason) = self.close_reason {
            m.insert("close_reason".into(), close_reason.clone());
        }
        m
    }

    fn doc_title(&self) -> Option<&str> {
        Some(&self.title)
    }

    fn doc_embedding_text(&self) -> String {
        // Include title, description, design, and acceptance criteria for semantic search
        let mut parts = vec![self.title.clone()];
        if !self.description.is_empty() {
            parts.push(self.description.clone());
        }
        if !self.design.is_empty() {
            parts.push(self.design.clone());
        }
        if !self.acceptance_criteria.is_empty() {
            parts.push(self.acceptance_criteria.clone());
        }
        parts.join("\n\n")
    }
}

// =============================================================================
// Rule implementation
// =============================================================================

impl SearchDocument for Rule {
    fn doc_id(&self) -> &str {
        &self.id
    }

    fn doc_content(&self) -> &str {
        &self.content
    }

    fn doc_type(&self) -> &str {
        "rule"
    }

    fn doc_tags(&self) -> Vec<&str> {
        self.tags.iter().map(|s| s.as_str()).collect()
    }

    fn doc_metadata(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("status".into(), format!("{:?}", self.status));
        m.insert("scope".into(), self.scope.to_string());
        m.insert("helpful_count".into(), self.helpful_count.to_string());
        m.insert("harmful_count".into(), self.harmful_count.to_string());
        if !self.paths.is_empty() {
            m.insert("paths".into(), self.paths.clone());
        }
        m
    }
}

// =============================================================================
// Skill implementation
// =============================================================================

impl SearchDocument for Skill {
    fn doc_id(&self) -> &str {
        &self.id
    }

    fn doc_content(&self) -> &str {
        &self.description
    }

    fn doc_type(&self) -> &str {
        "skill"
    }

    fn doc_tags(&self) -> Vec<&str> {
        self.tags.iter().map(|s| s.as_str()).collect()
    }

    fn doc_metadata(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("name".into(), self.name.clone());
        m.insert("scope".into(), self.scope.to_string());
        m.insert("status".into(), self.status.to_string());
        m.insert("invokable".into(), self.invokable.to_string());
        m.insert("usage_count".into(), self.usage_count.to_string());
        if !self.invocation.is_empty() {
            m.insert("invocation".into(), self.invocation.clone());
        }
        if !self.summary.is_empty() {
            m.insert("summary".into(), self.summary.clone());
        }
        if let Some(ref agent_type) = self.agent_type {
            m.insert("agent_type".into(), agent_type.clone());
        }
        m
    }

    fn doc_title(&self) -> Option<&str> {
        Some(&self.name)
    }

    fn doc_embedding_text(&self) -> String {
        // Include name, summary (if present), and description for semantic search
        let mut parts = vec![self.name.clone()];
        if !self.summary.is_empty() {
            parts.push(self.summary.clone());
        }
        parts.push(self.description.clone());
        parts.join("\n\n")
    }
}

// =============================================================================
// Spec implementation
// =============================================================================

impl SearchDocument for Spec {
    fn doc_id(&self) -> &str {
        &self.id
    }

    fn doc_content(&self) -> &str {
        // Return summary as primary content, fallback to title
        if !self.summary.is_empty() {
            &self.summary
        } else {
            &self.title
        }
    }

    fn doc_type(&self) -> &str {
        "spec"
    }

    fn doc_tags(&self) -> Vec<&str> {
        self.tags.iter().map(|s| s.as_str()).collect()
    }

    fn doc_metadata(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("title".into(), self.title.clone());
        m.insert("spec_type".into(), self.spec_type.to_string());
        m.insert("status".into(), self.status.to_string());
        m.insert("scope".into(), self.scope.to_string());
        m.insert("version".into(), self.version.to_string());
        if !self.summary.is_empty() {
            m.insert("summary".into(), self.summary.clone());
        }
        if let Some(ref task_id) = self.task_id {
            m.insert("task_id".into(), task_id.clone());
        }
        if let Some(ref previous_version_id) = self.previous_version_id {
            m.insert("previous_version_id".into(), previous_version_id.clone());
        }
        if let Some(ref approved_by) = self.approved_by {
            m.insert("approved_by".into(), approved_by.clone());
        }
        if let Some(ref team_id) = self.team_id {
            m.insert("team_id".into(), team_id.clone());
        }
        m
    }

    fn doc_title(&self) -> Option<&str> {
        Some(&self.title)
    }

    fn doc_embedding_text(&self) -> String {
        // Include title, summary, goals, acceptance_criteria, and design_notes for semantic search
        let mut parts = vec![self.title.clone()];
        if !self.summary.is_empty() {
            parts.push(self.summary.clone());
        }
        if !self.goals.is_empty() {
            parts.push(self.goals.join("\n"));
        }
        if !self.acceptance_criteria.is_empty() {
            parts.push(self.acceptance_criteria.join("\n"));
        }
        if !self.design_notes.is_empty() {
            parts.push(self.design_notes.clone());
        }
        parts.join("\n\n")
    }
}

// =============================================================================
// CodeSymbol implementation
// =============================================================================

impl SearchDocument for CodeSymbol {
    fn doc_id(&self) -> &str {
        &self.id
    }

    fn doc_content(&self) -> &str {
        &self.source
    }

    fn doc_type(&self) -> &str {
        "code_symbol"
    }

    fn doc_tags(&self) -> Vec<&str> {
        // CodeSymbol doesn't have tags, return empty
        Vec::new()
    }

    fn doc_metadata(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("name".into(), self.name.clone());
        m.insert("qualified_name".into(), self.qualified_name.clone());
        m.insert("kind".into(), format!("{:?}", self.kind));
        m.insert("language".into(), format!("{:?}", self.language));
        m.insert("file_path".into(), self.file_path.clone());
        m.insert("line_start".into(), self.line_start.to_string());
        m.insert("line_end".into(), self.line_end.to_string());
        m.insert("repository".into(), self.repository.clone());
        if let Some(ref signature) = self.signature {
            m.insert("signature".into(), signature.clone());
        }
        if let Some(ref documentation) = self.documentation {
            m.insert("documentation".into(), documentation.clone());
        }
        if let Some(ref parent_id) = self.parent_id {
            m.insert("parent_id".into(), parent_id.clone());
        }
        m
    }

    fn doc_title(&self) -> Option<&str> {
        Some(&self.qualified_name)
    }

    fn doc_embedding_text(&self) -> String {
        // Include qualified name, documentation (if present), signature (if present), and source
        let mut parts = vec![self.qualified_name.clone()];
        if let Some(ref documentation) = self.documentation {
            parts.push(documentation.clone());
        }
        if let Some(ref signature) = self.signature {
            parts.push(signature.clone());
        }
        // Only include first 500 chars of source to avoid huge embeddings
        let source_preview: String = self.source.chars().take(500).collect();
        parts.push(source_preview);
        parts.join("\n\n")
    }
}

#[cfg(test)]
#[path = "document_tests/tests.rs"]
mod tests;
