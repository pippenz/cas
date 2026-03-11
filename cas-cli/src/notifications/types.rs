//! Notification event types

use std::time::Instant;

/// Types of notification events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationEventType {
    // Task events
    TaskCreated,
    TaskStarted,
    TaskClosed,
    TaskUpdated,

    // Entry/memory events
    EntryAdded,
    EntryUpdated,
    EntryDeleted,

    // Rule events
    RuleCreated,
    RulePromoted,
    RuleDemoted,

    // Skill events
    SkillCreated,
    SkillEnabled,
    SkillDisabled,
}

impl NotificationEventType {
    /// Get a human-readable title for this event type
    pub fn title(&self) -> &'static str {
        match self {
            Self::TaskCreated => "Task Created",
            Self::TaskStarted => "Task Started",
            Self::TaskClosed => "Task Closed",
            Self::TaskUpdated => "Task Updated",
            Self::EntryAdded => "Memory Added",
            Self::EntryUpdated => "Memory Updated",
            Self::EntryDeleted => "Memory Deleted",
            Self::RuleCreated => "Rule Created",
            Self::RulePromoted => "Rule Promoted",
            Self::RuleDemoted => "Rule Demoted",
            Self::SkillCreated => "Skill Created",
            Self::SkillEnabled => "Skill Enabled",
            Self::SkillDisabled => "Skill Disabled",
        }
    }

    /// Get an icon/symbol for this event type
    pub fn icon(&self) -> &'static str {
        match self {
            Self::TaskCreated => "+",
            Self::TaskStarted => ">",
            Self::TaskClosed => "✓",
            Self::TaskUpdated => "~",
            Self::EntryAdded => "+",
            Self::EntryUpdated => "~",
            Self::EntryDeleted => "-",
            Self::RuleCreated => "+",
            Self::RulePromoted => "↑",
            Self::RuleDemoted => "↓",
            Self::SkillCreated => "+",
            Self::SkillEnabled => "✓",
            Self::SkillDisabled => "○",
        }
    }

    /// Check if this is a positive/success event (for color coding)
    pub fn is_positive(&self) -> bool {
        matches!(
            self,
            Self::TaskCreated
                | Self::TaskClosed
                | Self::EntryAdded
                | Self::RuleCreated
                | Self::RulePromoted
                | Self::SkillCreated
                | Self::SkillEnabled
        )
    }

    /// Check if this is a negative/warning event
    pub fn is_negative(&self) -> bool {
        matches!(
            self,
            Self::EntryDeleted | Self::RuleDemoted | Self::SkillDisabled
        )
    }
}

/// A notification event to display in the TUI
#[derive(Debug, Clone)]
pub struct NotificationEvent {
    /// Type of event
    pub event_type: NotificationEventType,
    /// ID of the affected entity
    pub entity_id: String,
    /// Short description/message
    pub message: String,
    /// When the event occurred (for timeout tracking)
    pub timestamp: Instant,
}

impl NotificationEvent {
    /// Create a new notification event
    pub fn new(
        event_type: NotificationEventType,
        entity_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            event_type,
            entity_id: entity_id.into(),
            message: message.into(),
            timestamp: Instant::now(),
        }
    }

    /// Get the title for display
    pub fn title(&self) -> &'static str {
        self.event_type.title()
    }

    /// Get the icon for display
    pub fn icon(&self) -> &'static str {
        self.event_type.icon()
    }

    // Convenience constructors for common events

    /// Task created notification
    pub fn task_created(id: &str, title: &str) -> Self {
        Self::new(NotificationEventType::TaskCreated, id, title.to_string())
    }

    /// Task started notification
    pub fn task_started(id: &str, title: &str) -> Self {
        Self::new(NotificationEventType::TaskStarted, id, title.to_string())
    }

    /// Task closed notification
    pub fn task_closed(id: &str, title: &str) -> Self {
        Self::new(NotificationEventType::TaskClosed, id, title.to_string())
    }

    /// Task updated notification
    pub fn task_updated(id: &str, title: &str) -> Self {
        Self::new(NotificationEventType::TaskUpdated, id, title.to_string())
    }

    /// Entry added notification
    pub fn entry_added(id: &str, entry_type: &str) -> Self {
        Self::new(
            NotificationEventType::EntryAdded,
            id,
            format!("New {entry_type} entry"),
        )
    }

    /// Entry updated notification
    pub fn entry_updated(id: &str) -> Self {
        Self::new(NotificationEventType::EntryUpdated, id, "Entry updated")
    }

    /// Entry deleted notification
    pub fn entry_deleted(id: &str) -> Self {
        Self::new(NotificationEventType::EntryDeleted, id, "Entry deleted")
    }

    /// Rule created notification
    pub fn rule_created(id: &str) -> Self {
        Self::new(NotificationEventType::RuleCreated, id, "New rule created")
    }

    /// Rule promoted notification
    pub fn rule_promoted(id: &str) -> Self {
        Self::new(
            NotificationEventType::RulePromoted,
            id,
            "Rule promoted to Proven",
        )
    }

    /// Rule demoted notification
    pub fn rule_demoted(id: &str) -> Self {
        Self::new(NotificationEventType::RuleDemoted, id, "Rule demoted")
    }

    /// Skill created notification
    pub fn skill_created(id: &str, name: &str) -> Self {
        Self::new(NotificationEventType::SkillCreated, id, name.to_string())
    }

    /// Skill enabled notification
    pub fn skill_enabled(id: &str, name: &str) -> Self {
        Self::new(NotificationEventType::SkillEnabled, id, name.to_string())
    }

    /// Skill disabled notification
    pub fn skill_disabled(id: &str, name: &str) -> Self {
        Self::new(NotificationEventType::SkillDisabled, id, name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::notifications::types::*;

    #[test]
    fn test_event_creation() {
        let event = NotificationEvent::task_created("cas-123", "Fix the bug");
        assert_eq!(event.event_type, NotificationEventType::TaskCreated);
        assert_eq!(event.entity_id, "cas-123");
        assert_eq!(event.message, "Fix the bug");
        assert_eq!(event.title(), "Task Created");
        assert_eq!(event.icon(), "+");
    }

    #[test]
    fn test_event_type_properties() {
        assert!(NotificationEventType::TaskClosed.is_positive());
        assert!(!NotificationEventType::TaskClosed.is_negative());

        assert!(!NotificationEventType::EntryDeleted.is_positive());
        assert!(NotificationEventType::EntryDeleted.is_negative());

        assert!(!NotificationEventType::TaskUpdated.is_positive());
        assert!(!NotificationEventType::TaskUpdated.is_negative());
    }
}
