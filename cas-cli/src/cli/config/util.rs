use crate::config::Constraint;

pub(crate) fn format_constraint(constraint: &Constraint) -> String {
    match constraint {
        Constraint::None => String::new(),
        Constraint::Min(min) => format!("minimum: {min}"),
        Constraint::Max(max) => format!("maximum: {max}"),
        Constraint::Range(min, max) => format!("range: {min} to {max}"),
        Constraint::OneOf(options) => format!("one of: {}", options.join(", ")),
        Constraint::NotEmpty => "cannot be empty".to_string(),
        Constraint::ValidPath => "must be a valid path".to_string(),
    }
}

pub(crate) fn truncate_description(desc: &str, max_len: usize) -> String {
    if desc.len() <= max_len {
        desc.to_string()
    } else {
        let mut end = max_len.saturating_sub(3).min(desc.len());
        while end > 0 && !desc.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &desc[..end])
    }
}
