/// Generate a unique session ID for this CLI invocation
pub(crate) fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();

    format!("cli-{timestamp:x}")
}

/// Sanitize command arguments to avoid storing sensitive data
pub(crate) fn sanitize_args(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|arg| {
            // Truncate very long arguments
            if arg.len() > 200 {
                let mut end = 197;
                while end > 0 && !arg.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &arg[..end])
            } else {
                arg.clone()
            }
        })
        .collect()
}
