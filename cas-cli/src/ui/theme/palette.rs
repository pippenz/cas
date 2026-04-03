//! Semantic color palette - what colors mean in context

use ratatui::style::Color;

use crate::ui::theme::colors::ColorPalette;

/// Semantic color palette mapping colors to UI meanings
#[derive(Debug, Clone)]
pub struct Palette {
    pub colors: ColorPalette,

    // Core UI
    pub bg_primary: Color,
    pub bg_secondary: Color,
    pub bg_elevated: Color,
    pub bg_overlay: Color,

    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub text_disabled: Color,

    pub border_default: Color,
    pub border_focused: Color,
    pub border_muted: Color,

    // Interactive
    pub accent: Color,
    pub accent_dim: Color,
    pub selection_bg: Color,
    pub highlight_bg: Color,

    // Status indicators
    pub status_success: Color,
    pub status_warning: Color,
    pub status_error: Color,
    pub status_info: Color,
    pub status_neutral: Color,

    // Task status
    pub task_open: Color,
    pub task_in_progress: Color,
    pub task_blocked: Color,
    pub task_closed: Color,

    // Priority levels
    pub priority_critical: Color,
    pub priority_high: Color,
    pub priority_medium: Color,
    pub priority_low: Color,
    pub priority_backlog: Color,

    // Entity types
    pub entity_id: Color,
    pub entity_type: Color,
    pub entity_skill: Color,
    pub entity_rule: Color,
    pub entity_agent: Color,

    // Agent status
    pub agent_active: Color,
    pub agent_idle: Color,
    pub agent_dead: Color,

    // Skill status
    pub skill_enabled: Color,
    pub skill_disabled: Color,
    pub skill_draft: Color,

    // Rule status
    pub rule_draft: Color,
    pub rule_proven: Color,
    pub rule_stale: Color,
    pub rule_retired: Color,

    // Verification
    pub verification_approved: Color,
    pub verification_rejected: Color,
    pub verification_error: Color,
    pub verification_skipped: Color,

    // Worktree status
    pub worktree_active: Color,
    pub worktree_merged: Color,
    pub worktree_abandoned: Color,
    pub worktree_conflict: Color,
    pub worktree_removed: Color,
    pub worktree_orphaned: Color,

    // Feedback scores
    pub score_positive: Color,
    pub score_negative: Color,
    pub score_neutral: Color,

    // Dashboard metrics
    pub metric_value: Color,
    pub trend_positive: Color,
    pub trend_negative: Color,

    // Bar charts
    pub bar_default: Color,
    pub bar_success: Color,
    pub bar_warning: Color,
    pub bar_error: Color,
    pub bar_info: Color,

    // Heatmap
    pub heatmap_empty: Color,
    pub heatmap_low: Color,
    pub heatmap_medium: Color,
    pub heatmap_high: Color,
    pub heatmap_max: Color,

    // Contextual hints
    pub hint_key: Color,
    pub hint_key_primary: Color,
    pub hint_description: Color,
}

impl Palette {
    pub fn from_colors(colors: ColorPalette, is_dark: bool) -> Self {
        if is_dark {
            Self::dark_from_colors(colors)
        } else {
            Self::light_from_colors(colors)
        }
    }

    fn dark_from_colors(colors: ColorPalette) -> Self {
        Self {
            // Core UI
            bg_primary: colors.gray_900,
            bg_secondary: colors.gray_800,
            bg_elevated: colors.gray_700,
            bg_overlay: Color::Rgb(0, 0, 0),

            text_primary: colors.gray_100,
            text_secondary: colors.gray_300,
            text_muted: colors.gray_400,
            text_disabled: colors.gray_500,

            border_default: colors.gray_500,
            border_focused: colors.primary_400,
            border_muted: colors.gray_600,

            // Interactive
            accent: colors.primary_400,
            accent_dim: colors.primary_500,
            selection_bg: colors.gray_700,
            highlight_bg: colors.primary_500,

            // Status
            status_success: colors.success,
            status_warning: colors.warning,
            status_error: colors.error,
            status_info: colors.info,
            status_neutral: colors.gray_400,

            // Task status
            task_open: colors.gray_400,
            task_in_progress: colors.info,
            task_blocked: colors.warning,
            task_closed: colors.success,

            // Priority
            priority_critical: colors.error,
            priority_high: Color::Rgb(255, 120, 100),
            priority_medium: colors.warning,
            priority_low: colors.gray_400,
            priority_backlog: colors.gray_500,

            // Entities
            entity_id: colors.cyan,
            entity_type: colors.warning,
            entity_skill: colors.purple,
            entity_rule: colors.success,
            entity_agent: colors.info,

            // Agent status
            agent_active: colors.success,
            agent_idle: colors.warning,
            agent_dead: colors.error,

            // Skill status
            skill_enabled: colors.success,
            skill_disabled: colors.gray_500,
            skill_draft: colors.warning,

            // Rule status
            rule_draft: colors.gray_400,
            rule_proven: colors.success,
            rule_stale: colors.warning,
            rule_retired: colors.error,

            // Verification
            verification_approved: colors.success,
            verification_rejected: colors.error,
            verification_error: colors.purple,
            verification_skipped: colors.gray_500,

            // Worktree
            worktree_active: colors.success,
            worktree_merged: colors.cyan,
            worktree_abandoned: colors.error,
            worktree_conflict: colors.purple,
            worktree_removed: colors.gray_500,
            worktree_orphaned: colors.warning,

            // Feedback
            score_positive: colors.success,
            score_negative: colors.error,
            score_neutral: colors.gray_500,

            // Dashboard metrics
            metric_value: colors.gray_100,
            trend_positive: colors.success,
            trend_negative: colors.error,

            // Bar charts
            bar_default: colors.primary_400,
            bar_success: colors.success,
            bar_warning: colors.warning,
            bar_error: colors.error,
            bar_info: colors.info,

            // Heatmap
            heatmap_empty: colors.gray_800,
            heatmap_low: Color::Rgb(70, 140, 70),
            heatmap_medium: Color::Rgb(80, 160, 80),
            heatmap_high: Color::Rgb(100, 200, 100),
            heatmap_max: colors.success,

            // Contextual hints
            hint_key: colors.gray_400,
            hint_key_primary: colors.primary_400,
            hint_description: colors.gray_500,

            colors,
        }
    }

    // MAINTENANCE: light_from_colors mirrors dark_from_colors field-for-field.
    // The light palette works via ColorPalette inversion (gray_50↔gray_900 etc.).
    // When adding a new semantic color to dark_from_colors, you MUST also add it here.
    fn light_from_colors(colors: ColorPalette) -> Self {
        Self {
            // Core UI - inverted for light
            bg_primary: colors.gray_900,
            bg_secondary: colors.gray_800,
            bg_elevated: colors.gray_700,
            bg_overlay: Color::Rgb(255, 255, 255),

            text_primary: colors.gray_100,
            text_secondary: colors.gray_300,
            text_muted: colors.gray_500,
            text_disabled: colors.gray_600,

            border_default: colors.gray_600,
            border_focused: colors.primary_400,
            border_muted: colors.gray_600,

            // Interactive
            accent: colors.primary_400,
            accent_dim: colors.primary_500,
            selection_bg: colors.gray_700,
            highlight_bg: colors.primary_500,

            // Status
            status_success: colors.success,
            status_warning: colors.warning,
            status_error: colors.error,
            status_info: colors.info,
            status_neutral: colors.gray_400,

            // Task status
            task_open: colors.gray_400,
            task_in_progress: colors.info,
            task_blocked: colors.warning,
            task_closed: colors.success,

            // Priority
            priority_critical: colors.error,
            priority_high: Color::Rgb(200, 80, 60),
            priority_medium: colors.warning,
            priority_low: colors.gray_500,
            priority_backlog: colors.gray_500,

            // Entities
            entity_id: colors.cyan,
            entity_type: colors.warning,
            entity_skill: colors.purple,
            entity_rule: colors.success,
            entity_agent: colors.info,

            // Agent status
            agent_active: colors.success,
            agent_idle: colors.warning,
            agent_dead: colors.error,

            // Skill status
            skill_enabled: colors.success,
            skill_disabled: colors.gray_500,
            skill_draft: colors.warning,

            // Rule status
            rule_draft: colors.gray_400,
            rule_proven: colors.success,
            rule_stale: colors.warning,
            rule_retired: colors.error,

            // Verification
            verification_approved: colors.success,
            verification_rejected: colors.error,
            verification_error: colors.purple,
            verification_skipped: colors.gray_500,

            // Worktree
            worktree_active: colors.success,
            worktree_merged: colors.cyan,
            worktree_abandoned: colors.error,
            worktree_conflict: colors.purple,
            worktree_removed: colors.gray_500,
            worktree_orphaned: colors.warning,

            // Feedback
            score_positive: colors.success,
            score_negative: colors.error,
            score_neutral: colors.gray_500,

            // Dashboard metrics
            metric_value: colors.gray_900,
            trend_positive: colors.success,
            trend_negative: colors.error,

            // Bar charts
            bar_default: colors.primary_400,
            bar_success: colors.success,
            bar_warning: colors.warning,
            bar_error: colors.error,
            bar_info: colors.info,

            // Heatmap
            heatmap_empty: colors.gray_300,
            heatmap_low: Color::Rgb(150, 220, 150),
            heatmap_medium: Color::Rgb(100, 180, 100),
            heatmap_high: Color::Rgb(60, 140, 60),
            heatmap_max: colors.success,

            // Contextual hints
            hint_key: colors.gray_600,
            hint_key_primary: colors.primary_400,
            hint_description: colors.gray_500,

            colors,
        }
    }
}
