//! Input mode handling for the factory TUI

/// The current input mode
#[derive(Debug, Clone, PartialEq, Default)]
pub enum InputMode {
    /// Normal mode - keys go to focused pane (supervisor only)
    #[default]
    Normal,
    /// Inject mode - typing into the inject buffer
    Inject,
    /// Pane select mode - hjkl/arrows navigate between panes, Enter confirms
    PaneSelect,
    /// Feedback mode - collecting user feedback
    Feedback,
    /// Resize mode - hjkl/arrows adjust pane sizes
    Resize,
    /// Terminal mode - interactive shell dialog
    Terminal,
}

impl InputMode {
    /// Check if currently in pane select mode
    pub fn is_pane_select(&self) -> bool {
        matches!(self, Self::PaneSelect)
    }

    /// Check if currently in feedback mode
    pub fn is_feedback(&self) -> bool {
        matches!(self, Self::Feedback)
    }

    /// Check if currently in resize mode
    pub fn is_resize(&self) -> bool {
        matches!(self, Self::Resize)
    }

    /// Check if currently in terminal mode
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminal)
    }
}

/// Layout size percentages for pane resizing
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutSizes {
    /// Workers percentage (0-100)
    pub workers: u16,
    /// Supervisor percentage (0-100)
    pub supervisor: u16,
    /// Sidecar percentage (0-100)
    pub sidecar: u16,
}

impl Default for LayoutSizes {
    fn default() -> Self {
        Self {
            workers: 50,
            supervisor: 30,
            sidecar: 20,
        }
    }
}

impl LayoutSizes {
    /// Minimum pane size percentage
    pub const MIN_SIZE: u16 = 10;
    /// Normal step size for resize operations
    pub const STEP: u16 = 5;
    /// Large step size for shift+key resize operations
    pub const LARGE_STEP: u16 = 10;

    /// Grow workers, shrink supervisor (respecting minimums)
    pub fn grow_workers(&mut self, step: u16) {
        if self.supervisor > Self::MIN_SIZE {
            let available = self.supervisor - Self::MIN_SIZE;
            let delta = step.min(available);
            self.workers += delta;
            self.supervisor -= delta;
        }
    }

    /// Shrink workers, grow supervisor (respecting minimums)
    pub fn shrink_workers(&mut self, step: u16) {
        if self.workers > Self::MIN_SIZE {
            let available = self.workers - Self::MIN_SIZE;
            let delta = step.min(available);
            self.workers -= delta;
            self.supervisor += delta;
        }
    }

    /// Grow supervisor, shrink sidecar (respecting minimums)
    pub fn grow_supervisor(&mut self, step: u16) {
        if self.sidecar > Self::MIN_SIZE {
            let available = self.sidecar - Self::MIN_SIZE;
            let delta = step.min(available);
            self.supervisor += delta;
            self.sidecar -= delta;
        }
    }

    /// Shrink supervisor, grow sidecar (respecting minimums)
    pub fn shrink_supervisor(&mut self, step: u16) {
        if self.supervisor > Self::MIN_SIZE {
            let available = self.supervisor - Self::MIN_SIZE;
            let delta = step.min(available);
            self.supervisor -= delta;
            self.sidecar += delta;
        }
    }

    /// Grow sidecar from supervisor (respecting minimums)
    pub fn grow_sidecar(&mut self, step: u16) {
        self.shrink_supervisor(step);
    }

    /// Shrink sidecar to supervisor (respecting minimums)
    pub fn shrink_sidecar(&mut self, step: u16) {
        self.grow_supervisor(step);
    }
}

/// Feedback category for user submissions
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FeedbackCategory {
    /// Bug report
    Bug,
    /// Feature suggestion
    #[default]
    Suggestion,
    /// Question about usage
    Question,
    /// Other feedback
    Other,
}

impl FeedbackCategory {
    /// Get all categories for UI display
    pub fn all() -> &'static [FeedbackCategory] {
        &[
            FeedbackCategory::Bug,
            FeedbackCategory::Suggestion,
            FeedbackCategory::Question,
            FeedbackCategory::Other,
        ]
    }

    /// Get the display name for this category
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedbackCategory::Bug => "Bug",
            FeedbackCategory::Suggestion => "Suggestion",
            FeedbackCategory::Question => "Question",
            FeedbackCategory::Other => "Other",
        }
    }

    /// Cycle to the next category
    pub fn next(&self) -> FeedbackCategory {
        match self {
            FeedbackCategory::Bug => FeedbackCategory::Suggestion,
            FeedbackCategory::Suggestion => FeedbackCategory::Question,
            FeedbackCategory::Question => FeedbackCategory::Other,
            FeedbackCategory::Other => FeedbackCategory::Bug,
        }
    }

    /// Cycle to the previous category
    pub fn prev(&self) -> FeedbackCategory {
        match self {
            FeedbackCategory::Bug => FeedbackCategory::Other,
            FeedbackCategory::Suggestion => FeedbackCategory::Bug,
            FeedbackCategory::Question => FeedbackCategory::Suggestion,
            FeedbackCategory::Other => FeedbackCategory::Question,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::factory::input::*;

    #[test]
    fn test_input_mode_default() {
        let mode = InputMode::default();
        assert!(matches!(mode, InputMode::Normal));
    }

    #[test]
    fn test_input_mode_is_pane_select() {
        assert!(!InputMode::Normal.is_pane_select());
        assert!(!InputMode::Inject.is_pane_select());
        assert!(InputMode::PaneSelect.is_pane_select());
        assert!(!InputMode::Feedback.is_pane_select());
        assert!(!InputMode::Resize.is_pane_select());
    }

    #[test]
    fn test_input_mode_is_feedback() {
        assert!(!InputMode::Normal.is_feedback());
        assert!(!InputMode::Inject.is_feedback());
        assert!(!InputMode::PaneSelect.is_feedback());
        assert!(InputMode::Feedback.is_feedback());
        assert!(!InputMode::Resize.is_feedback());
    }

    #[test]
    fn test_input_mode_is_resize() {
        assert!(!InputMode::Normal.is_resize());
        assert!(!InputMode::Inject.is_resize());
        assert!(!InputMode::PaneSelect.is_resize());
        assert!(!InputMode::Feedback.is_resize());
        assert!(InputMode::Resize.is_resize());
    }

    #[test]
    fn test_input_mode_is_terminal() {
        assert!(!InputMode::Normal.is_terminal());
        assert!(!InputMode::Inject.is_terminal());
        assert!(!InputMode::PaneSelect.is_terminal());
        assert!(!InputMode::Feedback.is_terminal());
        assert!(!InputMode::Resize.is_terminal());
        assert!(InputMode::Terminal.is_terminal());
    }

    #[test]
    fn test_layout_sizes_default() {
        let sizes = LayoutSizes::default();
        assert_eq!(sizes.workers, 50);
        assert_eq!(sizes.supervisor, 30);
        assert_eq!(sizes.sidecar, 20);
    }

    #[test]
    fn test_feedback_category_all() {
        let all = FeedbackCategory::all();
        assert_eq!(all.len(), 4);
        assert!(all.contains(&FeedbackCategory::Bug));
        assert!(all.contains(&FeedbackCategory::Suggestion));
        assert!(all.contains(&FeedbackCategory::Question));
        assert!(all.contains(&FeedbackCategory::Other));
    }

    #[test]
    fn test_feedback_category_next() {
        assert_eq!(FeedbackCategory::Bug.next(), FeedbackCategory::Suggestion);
        assert_eq!(
            FeedbackCategory::Suggestion.next(),
            FeedbackCategory::Question
        );
        assert_eq!(FeedbackCategory::Question.next(), FeedbackCategory::Other);
        assert_eq!(FeedbackCategory::Other.next(), FeedbackCategory::Bug);
    }

    #[test]
    fn test_feedback_category_prev() {
        assert_eq!(FeedbackCategory::Bug.prev(), FeedbackCategory::Other);
        assert_eq!(FeedbackCategory::Suggestion.prev(), FeedbackCategory::Bug);
        assert_eq!(
            FeedbackCategory::Question.prev(),
            FeedbackCategory::Suggestion
        );
        assert_eq!(FeedbackCategory::Other.prev(), FeedbackCategory::Question);
    }
}
