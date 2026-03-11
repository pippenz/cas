mod agent_coordination;
mod imports;
mod knowledge;
mod maintenance;
mod memory;
mod rules;
mod search;
mod skills;
mod system;
mod task;
mod task_extensions;
mod workflow;

/// Helper to truncate strings for display
pub(super) fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len.min(s.len());
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_str;

    #[test]
    fn truncate_str_handles_unicode_boundary() {
        let value = format!("{}✅ trailing", "a".repeat(99));
        assert_eq!(truncate_str(&value, 100), format!("{}...", "a".repeat(99)));
    }

    #[test]
    fn truncate_str_keeps_short_values() {
        assert_eq!(truncate_str("short", 10), "short");
    }
}
