//! Core markdown renderer using pulldown-cmark

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::ui::markdown::table::TableBuilder;
use crate::ui::theme::ActiveTheme;

/// Context for list rendering
#[derive(Debug, Clone)]
struct ListContext {
    ordered: bool,
    item_number: usize,
    indent_level: usize,
}

/// Markdown to ratatui renderer
pub struct MarkdownRenderer<'a> {
    theme: &'a ActiveTheme,

    // Output
    lines: Vec<Line<'static>>,

    // Current line building
    current_spans: Vec<Span<'static>>,

    // Style tracking
    style_stack: Vec<Style>,

    // List state
    list_stack: Vec<ListContext>,
    pending_list_prefix: Option<String>,

    // Code block state
    in_code_block: bool,
    code_block_content: Vec<String>,

    // Table state
    table_builder: Option<TableBuilder>,
    in_table_head: bool,

    // Blockquote state
    blockquote_depth: usize,
}

impl<'a> MarkdownRenderer<'a> {
    pub fn new(theme: &'a ActiveTheme) -> Self {
        Self {
            theme,
            lines: Vec::new(),
            current_spans: Vec::new(),
            style_stack: Vec::new(),
            list_stack: Vec::new(),
            pending_list_prefix: None,
            in_code_block: false,
            code_block_content: Vec::new(),
            table_builder: None,
            in_table_head: false,
            blockquote_depth: 0,
        }
    }

    pub fn render(mut self, content: &str) -> Vec<Line<'static>> {
        let options =
            Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;

        let parser = Parser::new_ext(content, options);

        for event in parser {
            self.process_event(event);
        }

        // Flush any remaining content
        self.flush_line();

        self.lines
    }

    fn process_event(&mut self, event: Event) {
        match event {
            // Block-level elements
            Event::Start(Tag::Heading { level, .. }) => self.start_heading(level),
            Event::End(TagEnd::Heading(_)) => self.end_heading(),

            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => self.end_paragraph(),

            Event::Start(Tag::CodeBlock(kind)) => self.start_code_block(kind),
            Event::End(TagEnd::CodeBlock) => self.end_code_block(),

            Event::Start(Tag::List(first_item)) => self.start_list(first_item),
            Event::End(TagEnd::List(_)) => self.end_list(),

            Event::Start(Tag::Item) => self.start_list_item(),
            Event::End(TagEnd::Item) => self.end_list_item(),

            Event::Start(Tag::BlockQuote(_)) => self.start_blockquote(),
            Event::End(TagEnd::BlockQuote(_)) => self.end_blockquote(),

            Event::Start(Tag::Table(alignments)) => self.start_table(alignments),
            Event::End(TagEnd::Table) => self.end_table(),

            Event::Start(Tag::TableHead) => self.in_table_head = true,
            Event::End(TagEnd::TableHead) => self.in_table_head = false,

            Event::Start(Tag::TableRow) => self.start_table_row(),
            Event::End(TagEnd::TableRow) => self.end_table_row(),

            Event::Start(Tag::TableCell) => {}
            Event::End(TagEnd::TableCell) => self.end_table_cell(),

            // Inline elements
            Event::Start(Tag::Strong) => self.push_style(self.theme.styles.text_bold),
            Event::End(TagEnd::Strong) => self.pop_style(),

            Event::Start(Tag::Emphasis) => self.push_italic(),
            Event::End(TagEnd::Emphasis) => self.pop_style(),

            Event::Start(Tag::Strikethrough) => self.push_strikethrough(),
            Event::End(TagEnd::Strikethrough) => self.pop_style(),

            Event::Start(Tag::Link { dest_url, .. }) => self.start_link(&dest_url),
            Event::End(TagEnd::Link) => self.pop_style(),

            Event::Code(text) => self.add_inline_code(&text),
            Event::Text(text) => self.add_text(&text),
            Event::SoftBreak => self.add_text(" "),
            Event::HardBreak => self.flush_line(),
            Event::Rule => self.add_rule(),

            Event::TaskListMarker(checked) => self.add_task_marker(checked),

            // Ignore these for now
            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => {}
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(_) => {}
            Event::Start(Tag::FootnoteDefinition(_)) | Event::End(TagEnd::FootnoteDefinition) => {}
            Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock) => {}
            Event::Start(Tag::MetadataBlock(_)) | Event::End(TagEnd::MetadataBlock(_)) => {}
            Event::Start(Tag::DefinitionList)
            | Event::End(TagEnd::DefinitionList)
            | Event::Start(Tag::DefinitionListTitle)
            | Event::End(TagEnd::DefinitionListTitle)
            | Event::Start(Tag::DefinitionListDefinition)
            | Event::End(TagEnd::DefinitionListDefinition) => {}

            // Math (render as inline code for now)
            Event::InlineMath(text) => self.add_inline_code(&text),
            Event::DisplayMath(text) => {
                self.flush_line();
                self.add_inline_code(&text);
                self.flush_line();
            }

            // Superscript and subscript (render as plain text)
            Event::Start(Tag::Superscript) | Event::End(TagEnd::Superscript) => {}
            Event::Start(Tag::Subscript) | Event::End(TagEnd::Subscript) => {}
        }
    }

    // =========================================================================
    // Style Management
    // =========================================================================

    fn current_style(&self) -> Style {
        self.style_stack
            .last()
            .copied()
            .unwrap_or(self.theme.styles.text_primary)
    }

    fn push_style(&mut self, style: Style) {
        self.style_stack.push(style);
    }

    fn push_italic(&mut self) {
        let base = self.current_style();
        self.style_stack.push(base.add_modifier(Modifier::ITALIC));
    }

    fn push_strikethrough(&mut self) {
        let base = self.current_style();
        self.style_stack
            .push(base.add_modifier(Modifier::CROSSED_OUT));
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }

    // =========================================================================
    // Line Management
    // =========================================================================

    fn flush_line(&mut self) {
        if self.current_spans.is_empty() {
            return;
        }

        let spans = std::mem::take(&mut self.current_spans);
        self.lines.push(Line::from(spans));
    }

    fn add_empty_line(&mut self) {
        self.lines.push(Line::from(""));
    }

    fn add_text(&mut self, text: &str) {
        if self.in_code_block {
            // Accumulate code block content
            self.code_block_content.push(text.to_string());
            return;
        }

        if let Some(ref mut table) = self.table_builder {
            // Accumulate table cell content
            table.add_cell_text(text);
            return;
        }

        // Handle list prefix if pending
        if let Some(prefix) = self.pending_list_prefix.take() {
            self.current_spans
                .push(Span::styled(prefix, self.theme.styles.text_muted));
        }

        // Add blockquote prefix if at start of line and in blockquote
        if self.current_spans.is_empty() && self.blockquote_depth > 0 {
            let prefix = "│ ".repeat(self.blockquote_depth);
            self.current_spans
                .push(Span::styled(prefix, self.theme.styles.border_muted));
        }

        let style = self.current_style();
        self.current_spans
            .push(Span::styled(text.to_string(), style));
    }

    fn add_inline_code(&mut self, text: &str) {
        if let Some(ref mut table) = self.table_builder {
            table.add_cell_text(&format!("`{text}`"));
            return;
        }

        self.current_spans.push(Span::styled(
            format!(" {text} "),
            self.theme.styles.md_code_inline,
        ));
    }

    // =========================================================================
    // Headings
    // =========================================================================

    fn start_heading(&mut self, level: HeadingLevel) {
        let style = match level {
            HeadingLevel::H1 => self.theme.styles.md_h1,
            HeadingLevel::H2 => self.theme.styles.md_h2,
            _ => self.theme.styles.md_h3,
        };
        self.push_style(style);
    }

    fn end_heading(&mut self) {
        self.pop_style();
        self.flush_line();
        self.add_empty_line();
    }

    // =========================================================================
    // Paragraphs
    // =========================================================================

    fn end_paragraph(&mut self) {
        self.flush_line();
        self.add_empty_line();
    }

    // =========================================================================
    // Code Blocks
    // =========================================================================

    fn start_code_block(&mut self, _kind: CodeBlockKind) {
        self.in_code_block = true;
        self.code_block_content.clear();

        // Add top border for code block
        self.lines.push(Line::from(Span::styled(
            "───────────────────────────────────",
            self.theme.styles.border_muted,
        )));
    }

    fn end_code_block(&mut self) {
        self.in_code_block = false;

        // Join accumulated code content and split by lines
        let code = std::mem::take(&mut self.code_block_content).join("");

        for line in code.lines() {
            self.lines.push(Line::from(vec![
                Span::styled("  │ ", self.theme.styles.border_muted),
                Span::styled(line.to_string(), self.theme.styles.md_code_block),
            ]));
        }

        // Add bottom border
        self.lines.push(Line::from(Span::styled(
            "───────────────────────────────────",
            self.theme.styles.border_muted,
        )));

        self.add_empty_line();
    }

    // =========================================================================
    // Lists
    // =========================================================================

    fn start_list(&mut self, first_item: Option<u64>) {
        let indent_level = self.list_stack.len();

        self.list_stack.push(ListContext {
            ordered: first_item.is_some(),
            item_number: first_item.unwrap_or(1) as usize,
            indent_level,
        });
    }

    fn end_list(&mut self) {
        self.list_stack.pop();

        // Add spacing after top-level list
        if self.list_stack.is_empty() {
            self.add_empty_line();
        }
    }

    fn start_list_item(&mut self) {
        if let Some(ctx) = self.list_stack.last_mut() {
            let indent = "  ".repeat(ctx.indent_level);
            let prefix = if ctx.ordered {
                let num = ctx.item_number;
                ctx.item_number += 1;
                format!("{indent}{num}. ")
            } else {
                format!("{indent}• ")
            };
            self.pending_list_prefix = Some(prefix);
        }
    }

    fn end_list_item(&mut self) {
        self.flush_line();
    }

    // =========================================================================
    // Blockquotes
    // =========================================================================

    fn start_blockquote(&mut self) {
        self.blockquote_depth += 1;
        self.push_style(self.theme.styles.md_blockquote);
    }

    fn end_blockquote(&mut self) {
        self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
        self.pop_style();
        self.flush_line();
    }

    // =========================================================================
    // Tables
    // =========================================================================

    fn start_table(&mut self, alignments: Vec<pulldown_cmark::Alignment>) {
        self.table_builder = Some(TableBuilder::new(alignments));
    }

    fn start_table_row(&mut self) {
        if let Some(ref mut table) = self.table_builder {
            table.start_row();
        }
    }

    fn end_table_row(&mut self) {
        if let Some(ref mut table) = self.table_builder {
            table.end_row(self.in_table_head);
        }
    }

    fn end_table_cell(&mut self) {
        if let Some(ref mut table) = self.table_builder {
            table.end_cell();
        }
    }

    fn end_table(&mut self) {
        if let Some(table) = self.table_builder.take() {
            let table_lines = table.render(self.theme);
            self.lines.extend(table_lines);
            self.add_empty_line();
        }
    }

    // =========================================================================
    // Links
    // =========================================================================

    fn start_link(&mut self, _url: &str) {
        self.push_style(self.theme.styles.md_link);
    }

    // =========================================================================
    // Horizontal Rules
    // =========================================================================

    fn add_rule(&mut self) {
        self.flush_line();
        self.lines.push(Line::from(Span::styled(
            "────────────────────────────────────────────────────────────",
            self.theme.styles.border_muted,
        )));
        self.add_empty_line();
    }

    // =========================================================================
    // Task Lists
    // =========================================================================

    fn add_task_marker(&mut self, checked: bool) {
        let marker = if checked { "[x] " } else { "[ ] " };
        let style = if checked {
            self.theme.styles.text_success
        } else {
            self.theme.styles.text_muted
        };
        self.current_spans.push(Span::styled(marker, style));
    }
}
