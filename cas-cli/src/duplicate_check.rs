//! Startup self-check for multiple `cas` binaries on PATH.
//!
//! When a developer has multiple `cas` binaries on PATH (e.g., a stale
//! `/usr/local/bin/cas` alongside a fresh `~/.local/bin/cas`), shell PATH order
//! or absolute-path invocations can silently resolve to the stale copy. This
//! module scans PATH on startup and emits a single-line stderr warning when
//! duplicates with different mtimes are found.
//!
//! Gating (see [`should_run`]):
//! * Skipped for `hook`, `serve`, and `factory` subcommands — they must stay
//!   silent per `feedback_hook_performance`.
//! * Skipped when stderr is not a TTY unless `CAS_WARN_DUPLICATES=1`.
//! * Unconditionally silenced when `CAS_SUPPRESS_DUPLICATE_WARNING=1`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Subcommands that must stay silent (hooks, long-running servers, factory TUI).
const QUIET_SUBCOMMANDS: &[&str] = &["hook", "serve", "factory"];

/// Decide whether the duplicate check should run for this invocation.
///
/// `args` is the full argv (including argv[0]).
pub fn should_run(args: &[String]) -> bool {
    if std::env::var_os("CAS_SUPPRESS_DUPLICATE_WARNING").is_some() {
        return false;
    }

    // Skip for subcommands that must be silent or long-lived.
    if let Some(sub) = first_subcommand(args) {
        if QUIET_SUBCOMMANDS.contains(&sub.as_str()) {
            return false;
        }
    }

    // Force-on override bypasses the TTY gate.
    if std::env::var_os("CAS_WARN_DUPLICATES").is_some() {
        return true;
    }

    is_stderr_tty()
}

/// Return the first non-flag token after argv[0], if any.
fn first_subcommand(args: &[String]) -> Option<String> {
    for token in args.iter().skip(1) {
        if token.starts_with('-') {
            continue;
        }
        return Some(token.clone());
    }
    None
}

#[cfg(unix)]
fn is_stderr_tty() -> bool {
    // SAFETY: isatty takes an fd and has no preconditions.
    unsafe { libc::isatty(libc::STDERR_FILENO) != 0 }
}

#[cfg(not(unix))]
fn is_stderr_tty() -> bool {
    false
}

/// Scan PATH for executables named `cas` and return the unique list, preserving
/// PATH order. Symlinks are canonicalised so that `/usr/bin/cas -> /usr/local/bin/cas`
/// does not count as two distinct binaries.
pub fn find_cas_binaries_on_path() -> Vec<PathBuf> {
    let path_var = match std::env::var_os("PATH") {
        Some(v) => v,
        None => return Vec::new(),
    };

    let mut seen: Vec<PathBuf> = Vec::new();
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join("cas");
        if !is_executable_file(&candidate) {
            continue;
        }
        let canonical = std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
        if seen.iter().any(|p| p == &canonical) {
            continue;
        }
        // Store the original (user-visible) path, but de-dupe via canonical.
        if seen.iter().all(|p| {
            std::fs::canonicalize(p)
                .ok()
                .as_deref()
                .map(|c| c != canonical)
                .unwrap_or(true)
        }) {
            seen.push(candidate);
        }
    }
    seen
}

fn is_executable_file(p: &Path) -> bool {
    let meta = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// A warning about duplicate binaries. Returned by [`build_warning`] so callers
/// (and tests) can decide how to present it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateWarning {
    /// The binary the current process is running from (argv[0] resolved).
    pub active: PathBuf,
    /// Other `cas` binaries on PATH with different mtimes.
    pub stale: Vec<PathBuf>,
}

impl DuplicateWarning {
    pub fn render(&self) -> String {
        let stale = self
            .stale
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "warning: multiple cas binaries on PATH with different mtimes — active: {}, stale: {} (set CAS_SUPPRESS_DUPLICATE_WARNING=1 to silence)",
            self.active.display(),
            stale,
        )
    }
}

/// Given a list of `cas` binaries on PATH and the active binary path, build a
/// warning if any of the others have an mtime that differs from the active one.
/// Returns `None` if only one binary exists or all mtimes match.
pub fn build_warning(binaries: &[PathBuf], active: &Path) -> Option<DuplicateWarning> {
    if binaries.len() < 2 {
        return None;
    }
    let active_mtime = mtime_of(active)?;
    let mut stale = Vec::new();
    for bin in binaries {
        if same_path(bin, active) {
            continue;
        }
        let Some(m) = mtime_of(bin) else { continue };
        if m != active_mtime {
            stale.push(bin.clone());
        }
    }
    if stale.is_empty() {
        None
    } else {
        Some(DuplicateWarning {
            active: active.to_path_buf(),
            stale,
        })
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let ac = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let bc = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ac == bc
}

fn mtime_of(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// Best-effort resolution of the currently-executing binary. Falls back to
/// `argv[0]` when `current_exe` fails (e.g., on exotic filesystems).
fn resolve_active_binary(argv0: &OsStr) -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from(argv0))
}

/// Entry point: run the check and print the warning once.
///
/// Best-effort; any filesystem error silently suppresses the warning.
pub fn check_and_warn(args: &[String]) {
    if !should_run(args) {
        return;
    }
    let Some(argv0) = args.first() else {
        return;
    };
    let active = resolve_active_binary(OsStr::new(argv0));
    let binaries = find_cas_binaries_on_path();
    if let Some(w) = build_warning(&binaries, &active) {
        eprintln!("{}", w.render());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    #[test]
    fn should_run_skips_hook_subcommand() {
        let args = vec!["cas".to_string(), "hook".to_string(), "PreToolUse".to_string()];
        // Even if a TTY would say yes, the subcommand veto wins.
        // Force-enable via env to isolate the subcommand gate.
        // SAFETY: test-only single-threaded env write.
        unsafe {
            std::env::set_var("CAS_WARN_DUPLICATES", "1");
            std::env::remove_var("CAS_SUPPRESS_DUPLICATE_WARNING");
        }
        assert!(!should_run(&args));
    }

    #[test]
    fn should_run_respects_suppress_env() {
        let args = vec!["cas".to_string(), "memory".to_string()];
        unsafe {
            std::env::set_var("CAS_SUPPRESS_DUPLICATE_WARNING", "1");
        }
        assert!(!should_run(&args));
        unsafe {
            std::env::remove_var("CAS_SUPPRESS_DUPLICATE_WARNING");
        }
    }

    #[test]
    fn first_subcommand_ignores_flags() {
        let args = vec![
            "cas".to_string(),
            "--verbose".to_string(),
            "task".to_string(),
            "list".to_string(),
        ];
        assert_eq!(first_subcommand(&args).as_deref(), Some("task"));
    }

    #[test]
    fn build_warning_none_for_single_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("cas");
        fs::write(&a, b"#!/bin/sh\n").unwrap();
        assert!(build_warning(std::slice::from_ref(&a), &a).is_none());
    }

    fn set_mtime(p: &Path, t: SystemTime) {
        let f = fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(t).unwrap();
    }

    #[test]
    fn build_warning_none_when_mtimes_match() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a_cas");
        let b = tmp.path().join("b_cas");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let t = SystemTime::now();
        set_mtime(&a, t);
        set_mtime(&b, t);
        assert!(build_warning(&[a.clone(), b.clone()], &a).is_none());
    }

    #[test]
    fn build_warning_flags_differing_mtimes() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a_cas");
        let b = tmp.path().join("b_cas");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let t = SystemTime::now();
        set_mtime(&a, t);
        set_mtime(&b, t - Duration::from_secs(3600));
        let w = build_warning(&[a.clone(), b.clone()], &a).expect("expected warning");
        assert_eq!(w.active, a);
        assert_eq!(w.stale, vec![b]);
        let rendered = w.render();
        assert!(rendered.contains("active:"));
        assert!(rendered.contains("stale:"));
        assert!(rendered.contains("CAS_SUPPRESS_DUPLICATE_WARNING"));
    }
}
