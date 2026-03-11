use crate::worktree::git::*;
use tempfile::TempDir;

fn create_test_repo() -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().to_path_buf();

    // Initialize git repo
    Command::new("git")
        .args(["init"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Create initial commit
    std::fs::write(repo_path.join("README.md"), "# Test").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    (temp, repo_path)
}

#[test]
fn test_git_available() {
    // Git should be available in test environment
    assert!(GitOperations::is_git_available());
}

#[test]
fn test_detect_repo_root() {
    let (_temp, repo_path) = create_test_repo();

    let detected = GitOperations::detect_repo_root(&repo_path).unwrap();
    // Canonicalize both paths to handle macOS /var -> /private/var symlinks
    let detected_canon = detected.canonicalize().unwrap_or(detected);
    let repo_canon = repo_path.canonicalize().unwrap_or(repo_path);
    assert_eq!(detected_canon, repo_canon);
}

#[test]
fn test_current_branch() {
    let (_temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path);

    let branch = git.current_branch().unwrap();
    // Default branch is usually "main" or "master"
    assert!(branch == "main" || branch == "master");
}

#[test]
fn test_create_and_remove_worktree() {
    let (temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path);

    let worktree_path = temp.path().join("feature-branch");

    // Create worktree
    git.create_worktree(&worktree_path, "feature-branch", None)
        .unwrap();
    assert!(worktree_path.exists());

    // List worktrees
    let worktrees = git.list_worktrees().unwrap();
    assert!(worktrees.len() >= 2); // Main + new worktree

    // Remove worktree
    git.remove_worktree(&worktree_path, false).unwrap();
    assert!(!worktree_path.exists());
}

#[test]
fn test_branch_exists() {
    let (_temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path);

    let current = git.current_branch().unwrap();
    assert!(git.branch_exists(&current).unwrap());
    assert!(!git.branch_exists("nonexistent-branch").unwrap());
}

#[test]
fn test_get_context() {
    let (_temp, repo_path) = create_test_repo();

    let context = GitOperations::get_context(&repo_path).unwrap();
    assert!(context.branch.is_some());
    assert!(!context.is_worktree); // Main checkout is not a worktree
}

#[test]
fn test_init_submodules_no_submodules() {
    // Test that init_submodules succeeds when there are no submodules
    let (_temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path.clone());

    // Should succeed silently when no .gitmodules exists
    let result = git.init_submodules(&repo_path);
    assert!(result.is_ok());
}

#[test]
fn test_init_submodules_with_gitmodules() {
    // Test that init_submodules runs when .gitmodules exists
    let (temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path.clone());

    // Create a .gitmodules file (simulating a repo with submodules)
    std::fs::write(
        repo_path.join(".gitmodules"),
        "[submodule \"vendor/test\"]\n\tpath = vendor/test\n\turl = https://example.com/test.git\n",
    )
    .unwrap();

    // Create a worktree
    let worktree_path = temp.path().join("test-worktree");
    git.create_worktree(&worktree_path, "test-branch", None)
        .unwrap();

    // The worktree should exist (submodule init may fail due to network,
    // but the worktree creation should still succeed)
    assert!(worktree_path.exists());
}

#[test]
fn test_worktree_with_submodule_init() {
    // Test that create_worktree calls init_submodules
    let (temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path.clone());

    let worktree_path = temp.path().join("sub-test");

    // Create worktree (should also init submodules if any)
    git.create_worktree(&worktree_path, "sub-test-branch", None)
        .unwrap();

    assert!(worktree_path.exists());

    // Verify we can manually call init_submodules again (idempotent)
    let result = git.init_submodules(&worktree_path);
    assert!(result.is_ok());
}

#[test]
fn test_get_submodule_paths() {
    let (_temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path.clone());

    // No .gitmodules - should return empty vec
    assert!(git.get_submodule_paths().unwrap().is_empty());

    // Create .gitmodules with submodule paths
    std::fs::write(
            repo_path.join(".gitmodules"),
            "[submodule \"vendor/ghostty\"]\n\tpath = vendor/ghostty\n\turl = https://example.com/ghostty.git\n\
             [submodule \"vendor/other\"]\n\tpath = vendor/other\n\turl = https://example.com/other.git\n",
        )
        .unwrap();

    let paths = git.get_submodule_paths().unwrap();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0], std::path::PathBuf::from("vendor/ghostty"));
    assert_eq!(paths[1], std::path::PathBuf::from("vendor/other"));
}

#[test]
fn test_mark_config_skip_worktree() {
    let (temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path.clone());

    // Create .claude directory with tracked files
    std::fs::create_dir_all(repo_path.join(".claude/rules")).unwrap();
    std::fs::write(repo_path.join(".claude/rules/test.md"), "test rule").unwrap();
    std::fs::write(repo_path.join("CLAUDE.md"), "# Claude").unwrap();

    Command::new("git")
        .args(["add", ".claude/", "CLAUDE.md"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Add config files"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Create a worktree
    let worktree_path = temp.path().join("test-skip-wt");
    git.create_worktree(&worktree_path, "test-skip-branch", None)
        .unwrap();

    // Mark skip-worktree
    git.mark_config_skip_worktree(&worktree_path).unwrap();

    // Modify a tracked file in the worktree
    std::fs::write(
        worktree_path.join(".claude/rules/test.md"),
        "modified rule content",
    )
    .unwrap();

    // The modification should NOT show up in git status
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&worktree_path)
        .output()
        .unwrap();

    let status = String::from_utf8_lossy(&output.stdout);
    assert!(
        !status.contains(".claude/rules/test.md"),
        "skip-worktree file should not appear in git status, got: {status}"
    );
}

#[test]
fn test_mark_config_skip_worktree_no_files() {
    let (_temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path.clone());

    // No .claude files tracked - should succeed silently
    let result = git.mark_config_skip_worktree(&repo_path);
    assert!(result.is_ok());
}

#[test]
fn test_fix_symlinked_submodules() {
    let (_temp, repo_path) = create_test_repo();
    let git = GitOperations::new(repo_path.clone());

    // Create .gitmodules
    std::fs::write(
        repo_path.join(".gitmodules"),
        "[submodule \"vendor/test\"]\n\tpath = vendor/test\n\turl = https://example.com/test.git\n",
    )
    .unwrap();

    // Create vendor directory
    std::fs::create_dir_all(repo_path.join("vendor")).unwrap();

    // Create a symlink for the submodule (simulating the workaround)
    let symlink_path = repo_path.join("vendor/test");
    let target = repo_path.join(".git"); // Just point to something that exists
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &symlink_path).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&target, &symlink_path).unwrap();

    assert!(symlink_path.is_symlink());

    // Fix should remove the symlink
    git.fix_symlinked_submodules(&repo_path).unwrap();

    // Symlink should be gone (submodule init may or may not succeed, but symlink is removed)
    assert!(!symlink_path.is_symlink());
}
