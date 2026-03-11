use crate::hooks::handlers::*;

pub fn generate_file_change_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let random: u32 = rand::random();
    format!("fc-{:x}-{:04x}", timestamp, random & 0xFFFF)
}

/// Compute content hash using SHA-256
pub fn compute_content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Get the repository name from the current directory
pub fn get_repository_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Normalize an absolute file path to a relative path
///
/// Enables cross-clone file tracking in factory mode by stripping the
/// clone-specific directory prefix. Handles both main repo and worktree/clone paths.
///
/// Examples:
/// - `/Users/user/project/src/foo.rs` -> `src/foo.rs` (main repo)
/// - `/Users/user/worktrees/swift-fox/src/foo.rs` -> `src/foo.rs` (clone)
pub fn normalize_to_relative_path(cas_root: &std::path::Path, file_path: &str) -> String {
    use std::path::Path;

    let path = Path::new(file_path);

    // If already relative, return as-is
    if path.is_relative() {
        return file_path.to_string();
    }

    // Try to strip the project root (parent of .cas directory)
    if let Some(project_root) = cas_root.parent() {
        if let Ok(relative) = path.strip_prefix(project_root) {
            return relative.to_string_lossy().to_string();
        }
    }

    // Try to strip current working directory (handles clone directories)
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(relative) = path.strip_prefix(&cwd) {
            return relative.to_string_lossy().to_string();
        }
    }

    // Fallback: return original path
    file_path.to_string()
}

/// Capture a file change for attribution tracking
///
/// Called from PostToolUse for Write and Edit tools.
/// Records which file was changed and links to the current prompt/session.
pub fn capture_file_change_for_attribution(
    cas_root: &std::path::Path,
    input: &HookInput,
    tool_name: &str,
) {
    // Get tool input
    let tool_input = match &input.tool_input {
        Some(ti) => ti,
        None => return,
    };

    // Extract file path and normalize to relative path
    let file_path_raw = match tool_input.get("file_path").and_then(|v| v.as_str()) {
        Some(fp) => fp,
        None => return,
    };

    // Normalize absolute paths to relative paths for cross-clone compatibility
    let file_path = normalize_to_relative_path(cas_root, file_path_raw);

    // Open the file change store
    let store = match open_file_change_store(cas_root) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Use session_id-based agent ID for attribution
    let agent_id = current_agent_id(input);

    // Get the most recent prompt for this session (for attribution linking)
    let prompt_id = get_current_prompt_id(cas_root, &input.session_id);

    // Determine change type and compute content hash
    let (change_type, old_content_hash, new_content_hash) = match tool_name {
        "Edit" => {
            let old_string = tool_input
                .get("old_string")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let new_string = tool_input
                .get("new_string")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let old_hash = if old_string.is_empty() {
                None
            } else {
                Some(compute_content_hash(old_string))
            };
            let new_hash = compute_content_hash(new_string);

            (ChangeType::Modified, old_hash, new_hash)
        }
        "Write" => {
            let content = tool_input
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let new_hash = compute_content_hash(content);

            (ChangeType::Created, None, new_hash)
        }
        _ => return,
    };

    let file_change = FileChange::with_prompt(
        generate_file_change_id(),
        input.session_id.clone(),
        agent_id,
        prompt_id,
        get_repository_name(),
        file_path.to_string(),
        change_type,
        tool_name.to_string(),
        old_content_hash,
        new_content_hash,
    );

    // Store silently - attribution is best-effort
    let _ = store.add(&file_change);
}

/// Get the most recent prompt ID for a session
pub fn get_current_prompt_id(cas_root: &std::path::Path, session_id: &str) -> Option<String> {
    let store = open_prompt_store(cas_root).ok()?;
    let prompts = store.list_by_session(session_id, 1).ok()?;
    prompts.into_iter().next().map(|p| p.id)
}

// =============================================================================
// GIT COMMIT DETECTION (Code Attribution)
// =============================================================================

/// Detect git commit command and link uncommitted file changes
///
/// Called from PostToolUse for Bash commands that contain "git commit".
/// Links all uncommitted file_changes for this session to the commit.
pub fn detect_and_link_git_commit(cas_root: &std::path::Path, input: &HookInput) {
    // Get tool input
    let tool_input = match &input.tool_input {
        Some(ti) => ti,
        None => return,
    };

    // Check if this is a git commit command
    let command = match tool_input.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd,
        None => return,
    };

    if !is_git_commit_command(command) {
        return;
    }

    // Get tool response to extract commit hash
    let tool_response = match &input.tool_response {
        Some(tr) => tr,
        None => return,
    };

    // Check for successful exit
    let exit_code = tool_response
        .get("exitCode")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    if exit_code != 0 {
        return; // Commit failed
    }

    // Extract commit hash from stdout
    let stdout = tool_response
        .get("stdout")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let commit_hash = match extract_commit_hash(stdout) {
        Some(hash) => hash,
        None => return, // Couldn't find commit hash
    };

    // Open stores
    let file_change_store = match open_file_change_store(cas_root) {
        Ok(s) => s,
        Err(_) => return,
    };

    let commit_link_store = match open_commit_link_store(cas_root) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Get uncommitted file changes for this session
    let uncommitted = match file_change_store.list_uncommitted(&input.session_id) {
        Ok(changes) => changes,
        Err(_) => return,
    };

    if uncommitted.is_empty() {
        return; // Nothing to link
    }

    // Extract metadata
    let agent_id = current_agent_id(input);
    let branch = get_current_branch().unwrap_or_else(|| "unknown".to_string());
    let message = extract_commit_message(command).unwrap_or_else(|| "No message".to_string());
    let author = get_git_author().unwrap_or_else(|| "Unknown".to_string());

    // Collect files and prompt IDs from uncommitted changes
    let files_changed: Vec<String> = uncommitted.iter().map(|c| c.file_path.clone()).collect();
    let prompt_ids: Vec<String> = uncommitted
        .iter()
        .filter_map(|c| c.prompt_id.clone())
        .collect();

    // Create commit link
    let commit_link = CommitLink::new(
        commit_hash.clone(),
        input.session_id.clone(),
        agent_id,
        branch,
        message,
        files_changed,
        prompt_ids,
        author,
    );

    // Store the commit link
    let _ = commit_link_store.add(&commit_link);

    // Link file changes to the commit
    let change_ids: Vec<String> = uncommitted.iter().map(|c| c.id.clone()).collect();
    let _ = file_change_store.link_to_commit(&change_ids, &commit_hash);
}

/// Check if a command is a git commit command
pub fn is_git_commit_command(command: &str) -> bool {
    let cmd_lower = command.to_lowercase();
    // Match "git commit" but not "git commit --amend" etc. that just show status
    cmd_lower.contains("git commit") && !cmd_lower.contains("--dry-run")
}

/// Extract commit hash from git commit output
///
/// Git commit output format: "[branch hash] message"
/// Example: "[main abc1234] Add new feature"
pub fn extract_commit_hash(stdout: &str) -> Option<String> {
    // Look for pattern: [branch hash] or just a commit hash line
    for line in stdout.lines() {
        let line = line.trim();

        // Format: [branch abc1234] message
        if line.starts_with('[') {
            if let Some(bracket_end) = line.find(']') {
                let inside = &line[1..bracket_end];
                // Split by space and get the hash (second word)
                let parts: Vec<&str> = inside.split_whitespace().collect();
                if parts.len() >= 2 {
                    let potential_hash = parts[1];
                    // Git short hash is typically 7+ chars, full is 40
                    if potential_hash.len() >= 7
                        && potential_hash.chars().all(|c| c.is_ascii_hexdigit())
                    {
                        return Some(potential_hash.to_string());
                    }
                }
            }
        }

        // Also check for full 40-char hash
        if line.len() == 40 && line.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(line.to_string());
        }
    }

    None
}

/// Extract commit message from git commit command
pub fn extract_commit_message(command: &str) -> Option<String> {
    // Look for -m "message" or -m 'message' pattern
    let patterns = ["-m \"", "-m '", "-m \"$(", "--message=\"", "--message='"];

    for pattern in patterns {
        if let Some(start) = command.find(pattern) {
            let msg_start = start + pattern.len();
            let quote_char = if pattern.contains('\'') { '\'' } else { '"' };

            // Find the closing quote
            let remaining = &command[msg_start..];
            if let Some(end) = remaining.find(quote_char) {
                return Some(remaining[..end].to_string());
            }
        }
    }

    // Try heredoc pattern: -m "$(cat <<'EOF'\nmessage\nEOF\n)"
    if command.contains("<<") {
        // Extract what's between heredoc markers
        if let Some(start) = command.find("<<") {
            let after_marker = &command[start + 2..];
            if let Some(marker_end) = after_marker.find('\n') {
                let marker = after_marker[..marker_end]
                    .trim()
                    .trim_matches('\'')
                    .trim_matches('"');
                let after_first_marker = &after_marker[marker_end + 1..];
                if let Some(msg_end) = after_first_marker.find(marker) {
                    let message = after_first_marker[..msg_end].trim();
                    if !message.is_empty() {
                        return Some(message.to_string());
                    }
                }
            }
        }
    }

    None
}

/// Get current git branch name
pub fn get_current_branch() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}

/// Get git author from config
pub fn get_git_author() -> Option<String> {
    let name = std::process::Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())?;

    let email = std::process::Command::new("git")
        .args(["config", "user.email"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())?;

    Some(format!("{name} <{email}>"))
}
