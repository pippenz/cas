//! File changes widget for sidecar and factory TUI
//!
//! Displays git file changes grouped by source (repo/worktree).
//! Supports hierarchical file tree display with collapsible directories.

use std::collections::{BTreeMap, HashSet};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState};

use crate::ui::theme::{ActiveTheme, Icons, Palette, get_agent_color};

// Re-export the DTO types from cas-factory for backward compatibility
pub use cas_factory::{FileChangeInfo, GitFileStatus, SourceChangesInfo};

/// Extension trait to add TUI-specific color method to GitFileStatus
pub trait GitFileStatusColor {
    fn color(&self, palette: &Palette) -> Color;
}

impl GitFileStatusColor for GitFileStatus {
    fn color(&self, palette: &Palette) -> Color {
        match self {
            Self::Modified => palette.status_warning,
            Self::Added => palette.status_success,
            Self::Deleted => palette.status_error,
            Self::Renamed => palette.status_info,
            Self::Untracked => palette.status_neutral,
        }
    }
}

/// Configuration for changes list rendering
#[derive(Debug, Default)]
pub struct ChangesConfig {
    /// Maximum files to show per source (0 = unlimited)
    pub max_files_per_source: usize,
    /// Whether to show line counts
    pub show_line_counts: bool,
    /// Whether to show staged indicator
    pub show_staged: bool,
    /// Whether to show hierarchical file tree
    pub show_hierarchy: bool,
}

impl ChangesConfig {
    pub fn new() -> Self {
        Self {
            max_files_per_source: 0, // Show all files
            show_line_counts: true,
            show_staged: true,
            show_hierarchy: true, // Enable hierarchy by default
        }
    }

    pub fn compact() -> Self {
        Self {
            max_files_per_source: 0, // Show all files
            show_line_counts: true,
            show_staged: false,
            show_hierarchy: true, // Enable hierarchy
        }
    }

    /// Flat list mode (legacy behavior)
    pub fn flat() -> Self {
        Self {
            max_files_per_source: 5,
            show_line_counts: true,
            show_staged: true,
            show_hierarchy: false,
        }
    }
}

/// Render a stateless changes list
pub fn render_changes_list(
    frame: &mut Frame,
    area: Rect,
    sources: &[SourceChangesInfo],
    theme: &ActiveTheme,
    config: &ChangesConfig,
    title: Option<&str>,
) {
    let palette = &theme.palette;
    let total: usize = sources.iter().map(|s| s.changes.len()).sum();
    let block = if let Some(title) = title {
        Block::default()
            .title(format!(" {title} ({total}) "))
            .borders(Borders::TOP)
            .border_style(Style::default().fg(palette.border_muted))
    } else {
        Block::default()
    };

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = build_changes_items(
        sources,
        theme,
        config,
        &HashSet::new(),
        inner.height as usize,
    );
    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Render a stateful changes list (for sidecar with selection)
pub fn render_changes_list_with_state(
    frame: &mut Frame,
    area: Rect,
    sources: &[SourceChangesInfo],
    theme: &ActiveTheme,
    config: &ChangesConfig,
    state: &mut ListState,
    block: Block,
) {
    render_changes_list_with_collapsed(
        frame,
        area,
        sources,
        theme,
        config,
        state,
        block,
        &HashSet::new(),
    )
}

/// Render a stateful changes list with collapsed directories
#[allow(clippy::too_many_arguments)]
pub fn render_changes_list_with_collapsed(
    frame: &mut Frame,
    area: Rect,
    sources: &[SourceChangesInfo],
    theme: &ActiveTheme,
    config: &ChangesConfig,
    state: &mut ListState,
    block: Block,
    collapsed_dirs: &HashSet<String>,
) {
    let styles = &theme.styles;
    let inner = block.inner(area);
    let items = build_changes_items(
        sources,
        theme,
        config,
        collapsed_dirs,
        inner.height as usize,
    );
    let list = List::new(items)
        .block(block)
        .highlight_style(styles.bg_selection);
    frame.render_stateful_widget(list, area, state);
}

/// Calculate the total number of visible lines for the changes list
pub fn calculate_changes_height(
    sources: &[SourceChangesInfo],
    config: &ChangesConfig,
    collapsed_dirs: &HashSet<String>,
) -> usize {
    if sources.is_empty() {
        return 1; // "No uncommitted changes" message
    }

    let mut total = 0;
    for source in sources {
        total += 1; // Source header

        if config.show_hierarchy {
            let tree = build_file_tree(&source.changes);
            total += tree.count_visible_lines(collapsed_dirs, &source.source_name);
        } else {
            let max_files = if config.max_files_per_source == 0 {
                source.changes.len()
            } else {
                config.max_files_per_source
            };
            total += max_files.min(source.changes.len());
            if config.max_files_per_source > 0 && source.changes.len() > config.max_files_per_source
            {
                total += 1; // "...and X more" line
            }
        }
    }
    total
}

/// Tree item type for tracking what each list item represents
#[derive(Debug, Clone)]
pub enum TreeItemType {
    /// Source header (source_name)
    Source(String),
    /// Directory (full_path)
    Directory(String),
    /// File (full_path)
    File(String),
}

/// Build changes list items and return the item types for navigation
pub fn build_changes_items_with_types(
    sources: &[SourceChangesInfo],
    theme: &ActiveTheme,
    config: &ChangesConfig,
    collapsed_dirs: &HashSet<String>,
    max_lines: usize,
) -> (Vec<ListItem<'static>>, Vec<TreeItemType>) {
    let palette = &theme.palette;
    let styles = &theme.styles;
    let mut items: Vec<ListItem> = Vec::new();
    let mut item_types: Vec<TreeItemType> = Vec::new();

    for source in sources {
        if items.len() >= max_lines {
            break;
        }

        let display_name = source.agent_name.as_ref().unwrap_or(&source.source_name);
        let source_color = get_agent_color(display_name);

        // Source header
        let header_text = if let Some(ref agent) = source.agent_name {
            format!("{} ({})", source.source_name, agent)
        } else {
            source.source_name.clone()
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                Icons::CIRCLE_FILLED.to_string(),
                Style::default().fg(source_color),
            ),
            Span::raw(" "),
            Span::styled(
                header_text,
                Style::default()
                    .fg(source_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({} files, ", source.changes.len()),
                styles.text_muted,
            ),
            Span::styled(
                format!("+{}", source.total_added),
                Style::default().fg(palette.status_success),
            ),
            Span::styled("/".to_string(), styles.text_muted),
            Span::styled(
                format!("-{}", source.total_removed),
                Style::default().fg(palette.status_error),
            ),
            Span::styled(")".to_string(), styles.text_muted),
        ])));
        item_types.push(TreeItemType::Source(source.source_name.clone()));

        if config.show_hierarchy {
            let tree = build_file_tree(&source.changes);
            tree.build_items_with_types(
                &mut items,
                &mut item_types,
                theme,
                config,
                source_color,
                "",
                true,
                collapsed_dirs,
                &source.source_name,
                max_lines,
            );
        } else {
            build_flat_items(
                &mut items,
                &mut item_types,
                theme,
                source,
                config,
                source_color,
                max_lines,
            );
        }
    }

    if items.is_empty() {
        items.push(ListItem::new(Line::from(vec![Span::styled(
            " No uncommitted changes",
            styles.text_muted.add_modifier(Modifier::ITALIC),
        )])));
        item_types.push(TreeItemType::Source(String::new()));
    }

    (items, item_types)
}

/// Node in the file tree
#[derive(Debug, Default)]
struct FileTreeNode {
    /// Child directories (sorted by name)
    children: BTreeMap<String, FileTreeNode>,
    /// Files in this directory
    files: Vec<FileChangeInfo>,
}

impl FileTreeNode {
    /// Insert a file into the tree at the given path
    fn insert(&mut self, path_parts: &[&str], change: FileChangeInfo) {
        if path_parts.len() == 1 {
            self.files.push(change);
        } else {
            let dir_name = path_parts[0].to_string();
            let child = self.children.entry(dir_name).or_default();
            child.insert(&path_parts[1..], change);
        }
    }

    /// Count visible lines (respecting collapsed state)
    fn count_visible_lines(&self, collapsed_dirs: &HashSet<String>, current_path: &str) -> usize {
        let mut count = 0;

        for (dir_name, child) in &self.children {
            count += 1; // Directory line
            let dir_path = if current_path.is_empty() {
                dir_name.clone()
            } else {
                format!("{current_path}/{dir_name}")
            };

            if !collapsed_dirs.contains(&dir_path) {
                count += child.count_visible_lines(collapsed_dirs, &dir_path);
            }
        }

        count += self.files.len();
        count
    }

    /// Build tree items with proper indentation and connectors
    #[allow(clippy::too_many_arguments)]
    fn build_items_with_types(
        &self,
        items: &mut Vec<ListItem<'static>>,
        item_types: &mut Vec<TreeItemType>,
        theme: &ActiveTheme,
        config: &ChangesConfig,
        source_color: Color,
        prefix: &str,
        _is_last_in_parent: bool,
        collapsed_dirs: &HashSet<String>,
        current_path: &str,
        max_lines: usize,
    ) {
        let palette = &theme.palette;
        let styles = &theme.styles;
        let dir_names: Vec<_> = self.children.keys().collect();
        let dir_count = dir_names.len();
        let file_count = self.files.len();

        // Render directories first
        for (i, dir_name) in dir_names.iter().enumerate() {
            if items.len() >= max_lines {
                return;
            }

            let is_last = i == dir_count - 1 && file_count == 0;
            let connector = if is_last { "└── " } else { "├── " };
            let child_prefix = if is_last { "    " } else { "│   " };

            let dir_path = if current_path.is_empty() {
                (*dir_name).clone()
            } else {
                format!("{current_path}/{dir_name}")
            };

            let is_collapsed = collapsed_dirs.contains(&dir_path);
            let collapse_icon = if is_collapsed {
                Icons::TRIANGLE_RIGHT
            } else {
                Icons::TRIANGLE_DOWN
            };

            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("   {prefix}{connector}"), styles.text_muted),
                Span::styled(format!("{collapse_icon} "), styles.text_muted),
                Span::styled(
                    format!("{dir_name}/"),
                    Style::default().fg(palette.accent_dim),
                ),
            ])));
            item_types.push(TreeItemType::Directory(dir_path.clone()));

            if !is_collapsed {
                if let Some(child) = self.children.get(*dir_name) {
                    child.build_items_with_types(
                        items,
                        item_types,
                        theme,
                        config,
                        source_color,
                        &format!("{prefix}{child_prefix}"),
                        is_last,
                        collapsed_dirs,
                        &dir_path,
                        max_lines,
                    );
                }
            }
        }

        // Render files
        for (i, change) in self.files.iter().enumerate() {
            if items.len() >= max_lines {
                return;
            }

            let is_last = i == file_count - 1;
            let connector = if is_last { "└── " } else { "├── " };
            let file_name = change
                .file_path
                .rsplit('/')
                .next()
                .unwrap_or(&change.file_path);

            let mut spans = vec![Span::styled(
                format!("   {prefix}{connector}"),
                styles.text_muted,
            )];

            if config.show_staged {
                let staged_indicator = if change.staged { "●" } else { "○" };
                spans.push(Span::styled(
                    staged_indicator.to_string(),
                    Style::default().fg(if change.staged {
                        palette.status_success
                    } else {
                        palette.status_neutral
                    }),
                ));
                spans.push(Span::raw(" "));
            }

            spans.push(Span::styled(
                change.status.symbol().to_string(),
                Style::default().fg(change.status.color(palette)),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                file_name.to_string(),
                Style::default().fg(source_color),
            ));

            if config.show_line_counts {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("+{}", change.lines_added),
                    Style::default().fg(palette.status_success),
                ));
                spans.push(Span::styled("/".to_string(), styles.text_muted));
                spans.push(Span::styled(
                    format!("-{}", change.lines_removed),
                    Style::default().fg(palette.status_error),
                ));
            }

            items.push(ListItem::new(Line::from(spans)));
            item_types.push(TreeItemType::File(change.file_path.clone()));
        }
    }
}

/// Build a file tree from a list of file changes
fn build_file_tree(changes: &[FileChangeInfo]) -> FileTreeNode {
    let mut root = FileTreeNode::default();
    for change in changes {
        let parts: Vec<&str> = change.file_path.split('/').collect();
        root.insert(&parts, change.clone());
    }
    root
}

/// Build flat list items (legacy mode)
fn build_flat_items(
    items: &mut Vec<ListItem<'static>>,
    item_types: &mut Vec<TreeItemType>,
    theme: &ActiveTheme,
    source: &SourceChangesInfo,
    config: &ChangesConfig,
    source_color: Color,
    max_lines: usize,
) {
    let palette = &theme.palette;
    let styles = &theme.styles;
    let max_files = if config.max_files_per_source == 0 {
        source.changes.len()
    } else {
        config.max_files_per_source
    };

    for change in source.changes.iter().take(max_files) {
        if items.len() >= max_lines {
            return;
        }

        let file_name = change
            .file_path
            .rsplit('/')
            .next()
            .unwrap_or(&change.file_path);

        let mut spans = vec![Span::raw("   ")];

        if config.show_staged {
            let staged_indicator = if change.staged { "●" } else { "○" };
            spans.push(Span::styled(
                staged_indicator.to_string(),
                Style::default().fg(if change.staged {
                    palette.status_success
                } else {
                    palette.status_neutral
                }),
            ));
            spans.push(Span::raw(" "));
        }

        spans.push(Span::styled(
            change.status.symbol().to_string(),
            Style::default().fg(change.status.color(palette)),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            file_name.to_string(),
            Style::default().fg(source_color),
        ));

        if config.show_line_counts {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("+{}", change.lines_added),
                Style::default().fg(palette.status_success),
            ));
            spans.push(Span::styled("/".to_string(), styles.text_muted));
            spans.push(Span::styled(
                format!("-{}", change.lines_removed),
                Style::default().fg(palette.status_error),
            ));
        }

        items.push(ListItem::new(Line::from(spans)));
        item_types.push(TreeItemType::File(change.file_path.clone()));
    }

    // Show "and X more" if there are more changes
    if config.max_files_per_source > 0
        && source.changes.len() > config.max_files_per_source
        && items.len() < max_lines
    {
        let more = source.changes.len() - config.max_files_per_source;
        items.push(ListItem::new(Line::from(vec![
            Span::raw("   "),
            Span::styled(
                format!("...and {more} more"),
                styles.text_muted.add_modifier(Modifier::ITALIC),
            ),
        ])));
        item_types.push(TreeItemType::Source(String::new()));
    }
}

/// Build changes list items (simple wrapper for backward compatibility)
fn build_changes_items(
    sources: &[SourceChangesInfo],
    theme: &ActiveTheme,
    config: &ChangesConfig,
    collapsed_dirs: &HashSet<String>,
    max_lines: usize,
) -> Vec<ListItem<'static>> {
    let (items, _) =
        build_changes_items_with_types(sources, theme, config, collapsed_dirs, max_lines);
    items
}

/// Render compact changes list for factory director
pub fn render_compact_changes_list(
    frame: &mut Frame,
    area: Rect,
    sources: &[SourceChangesInfo],
    theme: &ActiveTheme,
    focused: bool,
) {
    let _ = render_compact_changes_list_with_state(
        frame,
        area,
        sources,
        theme,
        focused,
        None,
        &HashSet::new(),
    );
}

/// Render compact changes list with optional scroll state and collapsed dirs
pub fn render_compact_changes_list_with_state(
    frame: &mut Frame,
    area: Rect,
    sources: &[SourceChangesInfo],
    theme: &ActiveTheme,
    focused: bool,
    state: Option<&mut ListState>,
    collapsed_dirs: &HashSet<String>,
) -> Vec<TreeItemType> {
    let styles = &theme.styles;
    let total: usize = sources.iter().map(|s| s.changes.len()).sum();
    let border_style = if focused {
        styles.border_focused
    } else {
        styles.border_default
    };

    let focus_marker = if focused { "▶" } else { " " };
    let block = Block::default()
        .title(format!(" {focus_marker} CHANGES ({total}) "))
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let config = ChangesConfig::compact();
    // Build ALL items - List widget handles scrolling
    let (items, item_types) =
        build_changes_items_with_types(sources, theme, &config, collapsed_dirs, usize::MAX);

    if items.is_empty() {
        let empty_list = List::new(vec![ListItem::new(Line::from(vec![Span::styled(
            "No changes",
            styles.text_muted.add_modifier(Modifier::ITALIC),
        )]))]);
        frame.render_widget(empty_list, inner);
    } else {
        let list = List::new(items).highlight_style(styles.bg_selection);
        if let Some(state) = state {
            frame.render_stateful_widget(list, inner, state);
        } else {
            frame.render_widget(list, inner);
        }
    }

    item_types
}
