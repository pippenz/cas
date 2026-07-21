#[cfg(test)]
mod tests {
    use super::super::{
        check_worktree_staleness, resolve_staleness_sync_ref, slugify_for_branch, truncate_str,
    };
    use std::process::Command;
    use tempfile::TempDir;

    // ========================================================================
    // Epic Branch Slugification Tests
    // ========================================================================

    #[test]
    fn test_slugify_for_branch_simple() {
        assert_eq!(slugify_for_branch("Add User Auth"), "add-user-auth");
        assert_eq!(slugify_for_branch("Simple Title"), "simple-title");
    }

    #[test]
    fn test_slugify_for_branch_special_chars() {
        assert_eq!(slugify_for_branch("Fix Bug #123"), "fix-bug-123");
        assert_eq!(slugify_for_branch("Add @feature!"), "add-feature");
        assert_eq!(
            slugify_for_branch("Special!@#$%^&*()Chars"),
            "special-chars"
        );
    }

    #[test]
    fn test_slugify_for_branch_multiple_spaces() {
        assert_eq!(slugify_for_branch("Multiple   Spaces"), "multiple-spaces");
        assert_eq!(
            slugify_for_branch("  Leading Trailing  "),
            "leading-trailing"
        );
    }

    #[test]
    fn test_slugify_for_branch_truncation() {
        // Test that long titles are truncated to 50 chars
        let long_title = "A".repeat(100);
        let result = slugify_for_branch(&long_title);
        assert_eq!(result.len(), 50);
        assert!(result.chars().all(|c| c == 'a'));
    }

    #[test]
    fn test_slugify_for_branch_preserves_numbers() {
        assert_eq!(
            slugify_for_branch("Version 2.0 Release"),
            "version-2-0-release"
        );
        assert_eq!(slugify_for_branch("CAS v1"), "cas-v1");
    }

    // ========================================================================
    // truncate_str Tests
    // ========================================================================

    #[test]
    fn truncate_str_handles_unicode_boundary() {
        let value = format!("{}✅ trailing", "a".repeat(99));
        assert_eq!(truncate_str(&value, 100), format!("{}...", "a".repeat(99)));
    }

    #[test]
    fn truncate_str_keeps_short_values() {
        assert_eq!(truncate_str("short", 10), "short");
    }

    // ========================================================================
    // Assignment freshness sync-ref resolution (cas-44e9)
    // ========================================================================

    #[test]
    fn resolve_staleness_preferred_wins_over_upstream_and_default() {
        let got = resolve_staleness_sync_ref(
            Some("epic/alpha"),
            "factory/worker",
            Some("origin/factory/worker"),
            "main",
        );
        assert_eq!(got, "epic/alpha");
    }

    #[test]
    fn resolve_staleness_preferred_wins_even_when_two_epics_would_exist() {
        // Regression: old code listed epic/* and took .last() — wrong under multi-epic.
        // Preferred (task parent epic A) must always win; never invent from listing.
        let got = resolve_staleness_sync_ref(
            Some("epic/a-first"),
            "factory/hv-scope",
            None,
            "main",
        );
        assert_eq!(got, "epic/a-first");
        assert_ne!(got, "epic/z-last");
    }

    #[test]
    fn resolve_staleness_factory_without_preferred_uses_default_not_epic_list() {
        // No parent epic / focus pin → base/main. Must not return an epic/* name.
        let got = resolve_staleness_sync_ref(None, "factory/worker", None, "main");
        assert_eq!(got, "main");
        assert!(!got.starts_with("epic/"));
    }

    #[test]
    fn resolve_staleness_empty_preferred_falls_through_to_upstream() {
        let got = resolve_staleness_sync_ref(
            Some("   "),
            "factory/worker",
            Some("origin/main"),
            "main",
        );
        assert_eq!(got, "origin/main");
    }

    #[test]
    fn resolve_staleness_epic_branch_without_preferred_uses_self() {
        let got = resolve_staleness_sync_ref(None, "epic/own", None, "main");
        assert_eq!(got, "epic/own");
    }

    fn git(path: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(path)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("git spawn");
        assert!(status.success(), "git {args:?} failed");
    }

    /// Multi-epic repo: epic/a and epic/z both exist; worker on factory/* is behind
    /// only epic/a. Preferred=epic/a must report that branch — never epic/z.
    #[test]
    fn check_worktree_staleness_uses_preferred_when_two_epic_branches_exist() {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("seed.txt"), "seed\n").unwrap();
        git(p, &["add", "seed.txt"]);
        git(p, &["commit", "-q", "-m", "seed"]);

        // epic/a: one extra commit (worker will be behind this)
        git(p, &["checkout", "-q", "-b", "epic/a"]);
        std::fs::write(p.join("a.txt"), "a\n").unwrap();
        git(p, &["add", "a.txt"]);
        git(p, &["commit", "-q", "-m", "epic a"]);

        // epic/z: two extra commits off main (would be wrong multi-epic pick if .last())
        git(p, &["checkout", "-q", "main"]);
        git(p, &["checkout", "-q", "-b", "epic/z"]);
        std::fs::write(p.join("z1.txt"), "z1\n").unwrap();
        git(p, &["add", "z1.txt"]);
        git(p, &["commit", "-q", "-m", "epic z1"]);
        std::fs::write(p.join("z2.txt"), "z2\n").unwrap();
        git(p, &["add", "z2.txt"]);
        git(p, &["commit", "-q", "-m", "epic z2"]);

        // factory worker branched from main (behind both epics)
        git(p, &["checkout", "-q", "main"]);
        git(p, &["checkout", "-q", "-b", "factory/worker"]);

        let path = p.to_str().unwrap();
        let (behind, branch) = check_worktree_staleness(path, Some("epic/a"))
            .expect("staleness check should succeed");
        assert_eq!(branch, "epic/a", "must name preferred epic A, not concurrent epic Z");
        assert!(!branch.contains("epic/z"), "wrong epic must not appear: {branch}");
        assert_eq!(behind, 1, "worker is 1 commit behind epic/a");

        // Without preferred: base/main — still must not invent epic/z via list.last()
        let (behind_main, branch_main) =
            check_worktree_staleness(path, None).expect("staleness without preferred");
        assert_eq!(branch_main, "main");
        assert_eq!(behind_main, 0);
        assert!(!branch_main.starts_with("epic/"));
    }

    #[test]
    fn check_worktree_staleness_missing_path_returns_none() {
        assert!(check_worktree_staleness("/nonexistent/worktree/path", Some("epic/a")).is_none());
    }
}
