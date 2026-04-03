use std::path::Path;

use crate::hooks::handlers::*;

/// Represents a structural file change detected from a git commit
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodemapChange {
    /// Change type: "A" (added), "D" (deleted), "R" (renamed)
    #[serde(rename = "type")]
    pub change_type: String,
    /// File path affected
    pub path: String,
    /// For renames, the old path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
}

/// A single pending codemap entry (JSONL format - one per line)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodemapPending {
    pub changes: Vec<CodemapChange>,
    pub commit: String,
    pub recorded_at: String,
}

/// Path to the codemap pending file relative to cas_root
const CODEMAP_PENDING_FILE: &str = "codemap-pending.json";

/// Detect structural file changes (add/delete/rename) from a successful git commit.
///
/// Called from PostToolUse for Bash commands containing "git commit" that exit 0.
/// Writes changes to `.cas/codemap-pending.json` in JSONL format (append-safe).
///
/// Pattern follows `detect_and_link_git_commit` in attribution.rs: same trigger,
/// same silent-failure error handling.
pub fn detect_codemap_structural_changes(cas_root: &Path, input: &HookInput) {
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

    if !super::attribution::is_git_commit_command(command) {
        return;
    }

    // Check for successful exit
    let tool_response = match &input.tool_response {
        Some(tr) => tr,
        None => return,
    };

    let exit_code = tool_response
        .get("exitCode")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    if exit_code != 0 {
        return;
    }

    // Get the commit hash from stdout
    let stdout = tool_response
        .get("stdout")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let commit_hash = match super::attribution::extract_commit_hash(stdout) {
        Some(hash) => hash,
        None => return,
    };

    // Run git diff-tree to find structural changes (A/D/R only)
    let changes = match get_structural_changes(&commit_hash) {
        Some(c) if !c.is_empty() => c,
        _ => return, // No structural changes or error
    };

    // Write to codemap-pending.json (JSONL append)
    let pending = CodemapPending {
        changes,
        commit: commit_hash,
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };

    let pending_path = cas_root.join(CODEMAP_PENDING_FILE);

    // Serialize as single-line JSON for JSONL format
    let line = match serde_json::to_string(&pending) {
        Ok(json) => format!("{json}\n"),
        Err(_) => return,
    };

    // Append to file (create if doesn't exist)
    use std::io::Write;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&pending_path);

    match file {
        Ok(mut f) => {
            let _ = f.write_all(line.as_bytes());
        }
        Err(_) => {} // Silent failure - best effort
    }
}

/// Run `git diff-tree --name-status -r HEAD~1 HEAD` and parse structural changes.
///
/// Only returns Added (A), Deleted (D), and Renamed (R) entries.
/// Returns None on git command failure.
fn get_structural_changes(commit_hash: &str) -> Option<Vec<CodemapChange>> {
    // Use the specific commit to diff against its parent
    let output = std::process::Command::new("git")
        .args([
            "diff-tree",
            "--name-status",
            "-r",
            "--no-commit-id",
            &format!("{commit_hash}~1"),
            commit_hash,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        // Fallback for initial commit (no parent)
        let output = std::process::Command::new("git")
            .args([
                "diff-tree",
                "--name-status",
                "-r",
                "--no-commit-id",
                "--root",
                commit_hash,
            ])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        return parse_diff_tree_output(&String::from_utf8_lossy(&output.stdout));
    }

    parse_diff_tree_output(&String::from_utf8_lossy(&output.stdout))
}

/// Parse `git diff-tree --name-status` output into structural changes.
///
/// Only keeps A (added), D (deleted), and R (renamed) entries.
/// Ignores M (modified) and other change types.
fn parse_diff_tree_output(output: &str) -> Option<Vec<CodemapChange>> {
    let mut changes = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }

        let status = parts[0];
        match status {
            "A" => {
                changes.push(CodemapChange {
                    change_type: "A".to_string(),
                    path: parts[1].to_string(),
                    old_path: None,
                });
            }
            "D" => {
                changes.push(CodemapChange {
                    change_type: "D".to_string(),
                    path: parts[1].to_string(),
                    old_path: None,
                });
            }
            s if s.starts_with('R') && parts.len() >= 3 => {
                // Rename: R100\told_path\tnew_path
                changes.push(CodemapChange {
                    change_type: "R".to_string(),
                    path: parts[2].to_string(),
                    old_path: Some(parts[1].to_string()),
                });
            }
            _ => {} // Ignore M (modified), C (copied), etc.
        }
    }

    Some(changes)
}

/// Check for codemap freshness and return context injection string.
///
/// Called from SessionStart to inform the agent about:
/// 1. Missing CODEMAP.md
/// 2. Pending structural changes since last codemap update
///
/// Returns None if no action needed.
pub fn check_codemap_freshness(cas_root: &Path) -> Option<String> {
    let project_root = cas_root.parent()?;

    let codemap_path = project_root.join(".claude/CODEMAP.md");
    let pending_path = cas_root.join(CODEMAP_PENDING_FILE);

    let codemap_exists = codemap_path.exists();
    let pending_exists = pending_path.exists();

    if codemap_exists && !pending_exists {
        return None; // CODEMAP exists and no pending changes
    }

    if !codemap_exists {
        return Some(
            "<codemap-freshness>\n\
             No CODEMAP.md found. Run the codemap skill to generate one.\n\
             </codemap-freshness>"
                .to_string(),
        );
    }

    // CODEMAP exists but there are pending changes
    if pending_exists {
        // Read pending file and count changes
        let content = std::fs::read_to_string(&pending_path).ok()?;
        let mut total_changes = 0;
        let mut file_list = Vec::new();
        let mut first_commit = None;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(pending) = serde_json::from_str::<CodemapPending>(line) {
                if first_commit.is_none() {
                    first_commit = Some(pending.commit.clone());
                }
                for change in &pending.changes {
                    total_changes += 1;
                    if file_list.len() < 10 {
                        let prefix = match change.change_type.as_str() {
                            "A" => "+",
                            "D" => "-",
                            "R" => "~",
                            _ => "?",
                        };
                        file_list.push(format!("{prefix}{}", change.path));
                    }
                }
            }
        }

        if total_changes == 0 {
            return None;
        }

        let commit_info = first_commit
            .map(|c| format!(" since {}", &c[..7.min(c.len())]))
            .unwrap_or_default();

        let files = file_list.join(", ");
        let truncated = if total_changes > 10 {
            format!(" (+{} more)", total_changes - 10)
        } else {
            String::new()
        };

        return Some(format!(
            "<codemap-freshness>\n\
             CODEMAP.md has {total_changes} pending structural change(s){commit_info}: {files}{truncated}. \
             Update CODEMAP.md or spawn docs-writer to refresh.\n\
             </codemap-freshness>"
        ));
    }

    None
}

/// Best-effort codemap reminder for Stop hook.
///
/// Returns a reminder string if there are pending structural changes.
pub fn codemap_stop_reminder(cas_root: &Path) -> Option<String> {
    let pending_path = cas_root.join(CODEMAP_PENDING_FILE);

    if !pending_path.exists() {
        return None;
    }

    // Count pending changes (best-effort)
    let content = std::fs::read_to_string(&pending_path).ok()?;
    let total: usize = content
        .lines()
        .filter_map(|line| serde_json::from_str::<CodemapPending>(line.trim()).ok())
        .map(|p| p.changes.len())
        .sum();

    if total == 0 {
        return None;
    }

    Some(format!(
        "Note: CODEMAP.md has {total} pending structural change(s) that should be updated."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_diff_tree_added() {
        let output = "A\tsrc/new_module.rs\n";
        let changes = parse_diff_tree_output(output).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, "A");
        assert_eq!(changes[0].path, "src/new_module.rs");
        assert!(changes[0].old_path.is_none());
    }

    #[test]
    fn test_parse_diff_tree_deleted() {
        let output = "D\tsrc/old_module.rs\n";
        let changes = parse_diff_tree_output(output).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, "D");
        assert_eq!(changes[0].path, "src/old_module.rs");
    }

    #[test]
    fn test_parse_diff_tree_renamed() {
        let output = "R100\tsrc/old_name.rs\tsrc/new_name.rs\n";
        let changes = parse_diff_tree_output(output).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, "R");
        assert_eq!(changes[0].path, "src/new_name.rs");
        assert_eq!(changes[0].old_path.as_deref(), Some("src/old_name.rs"));
    }

    #[test]
    fn test_parse_diff_tree_ignores_modified() {
        let output = "M\tsrc/existing.rs\nA\tsrc/new.rs\n";
        let changes = parse_diff_tree_output(output).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, "A");
    }

    #[test]
    fn test_parse_diff_tree_mixed() {
        let output = "A\tsrc/new.rs\nD\tsrc/old.rs\nM\tsrc/modified.rs\nR095\tsrc/a.rs\tsrc/b.rs\n";
        let changes = parse_diff_tree_output(output).unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].change_type, "A");
        assert_eq!(changes[1].change_type, "D");
        assert_eq!(changes[2].change_type, "R");
    }

    #[test]
    fn test_parse_diff_tree_empty() {
        let changes = parse_diff_tree_output("").unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn test_codemap_pending_serialization() {
        let pending = CodemapPending {
            changes: vec![
                CodemapChange {
                    change_type: "A".to_string(),
                    path: "src/new.rs".to_string(),
                    old_path: None,
                },
                CodemapChange {
                    change_type: "D".to_string(),
                    path: "src/old.rs".to_string(),
                    old_path: None,
                },
            ],
            commit: "abc1234".to_string(),
            recorded_at: "2026-04-03T18:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&pending).unwrap();
        let deserialized: CodemapPending = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.changes.len(), 2);
        assert_eq!(deserialized.commit, "abc1234");
    }

    #[test]
    fn test_codemap_pending_jsonl_parsing() {
        let jsonl = r#"{"changes":[{"type":"A","path":"src/new.rs"}],"commit":"abc1234","recorded_at":"2026-04-03T18:00:00Z"}
{"changes":[{"type":"D","path":"src/old.rs"}],"commit":"def5678","recorded_at":"2026-04-03T19:00:00Z"}"#;

        let entries: Vec<CodemapPending> = jsonl
            .lines()
            .filter_map(|line| serde_json::from_str(line.trim()).ok())
            .collect();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].commit, "abc1234");
        assert_eq!(entries[1].commit, "def5678");
    }

    #[test]
    fn test_check_codemap_freshness_no_cas_parent() {
        // cas_root with no parent should return None
        let result = check_codemap_freshness(Path::new("/"));
        // Root has no parent, returns None
        assert!(result.is_none());
    }

    #[test]
    fn test_codemap_stop_reminder_no_file() {
        let temp = std::env::temp_dir().join("test_codemap_stop_reminder");
        let _ = std::fs::create_dir_all(&temp);
        let result = codemap_stop_reminder(&temp);
        assert!(result.is_none());
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_codemap_stop_reminder_with_pending() {
        let temp = std::env::temp_dir().join("test_codemap_stop_with_pending");
        let _ = std::fs::create_dir_all(&temp);

        let pending_path = temp.join(CODEMAP_PENDING_FILE);
        std::fs::write(
            &pending_path,
            r#"{"changes":[{"type":"A","path":"src/new.rs"},{"type":"D","path":"src/old.rs"}],"commit":"abc1234","recorded_at":"2026-04-03T18:00:00Z"}"#,
        )
        .unwrap();

        let result = codemap_stop_reminder(&temp);
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(msg.contains("2 pending structural change(s)"));

        let _ = std::fs::remove_dir_all(&temp);
    }
}
