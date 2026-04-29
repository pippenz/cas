//! `cas integrate vercel <init|refresh|verify>` — Vercel platform handler.
//!
//! Implements EPIC cas-b65f Unit 2 (task **cas-8e37**) on top of the
//! foundation in [`super::keep_block`] + [`super::types`].
//!
//! # Stub UX convention (mirrored by neon + github)
//!
//! The foundation left "what's an `Err` vs `Ok(Skipped)`" to the first handler
//! to land. This handler sets the convention that **cas-1ece (neon)** and
//! **cas-f425 (github)** mirror:
//!
//! - **`Ok(IntegrationOutcome { status: Skipped, .. })`** when:
//!     - The platform is not detected in the repo (no `vercel.json`, no
//!       `@vercel/*` deps in `package.json`).
//!     - The user cancels the multi-match picker prompt.
//!     - `init` is run on a repo that already has a populated SKILL.md
//!       (status becomes `AlreadyConfigured`, with a hint to use
//!       `cas integrate vercel refresh`).
//! - **`Err(...)`** when:
//!     - The MCP proxy is unreachable, returns malformed data, or fails
//!       authentication.
//!     - The repo root cannot be located.
//!     - Filesystem I/O fails on a path the user expects to write.
//!
//! # MCP wiring
//!
//! The handler talks to Vercel through a [`VercelClient`] trait. Tests inject
//! [`MockVercelClient`]; production uses [`mcp_proxy_client::ProxyVercelClient`]
//! (gated behind the `mcp-proxy` feature). When the binary is not built with
//! `--features mcp-proxy`, calls fall through to a clear error pointing the
//! user at how to enable it.
//!
//! # Templates
//!
//! Templates live in `templates/vercel/` next to this file and are embedded
//! at compile time via `include_str!`. The mode hold all project-specific
//! IDs inside named keep blocks (`<!-- keep vercel-ids -->`,
//! `<!-- keep vercel-notes -->`) so refresh can either preserve them
//! (default) or replace them (`--update-ids`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

use super::keep_block::{self, MergeMode};
use super::types::{
    IntegrationAction, IntegrationOutcome, IntegrationStatus, Platform,
};

/// Embedded template content.
const TEMPLATE_CLAUDE_SKILL: &str = include_str!("templates/vercel/SKILL.md.tmpl");
const TEMPLATE_CURSOR_SKILL: &str = include_str!("templates/vercel/cursor.md.tmpl");
const TEMPLATE_COMMON_TASKS: &str =
    include_str!("templates/vercel/references/common-tasks.md");

const REL_CLAUDE_SKILL: &str = ".claude/skills/vercel-deployments/SKILL.md";
const REL_CLAUDE_REFS: &str =
    ".claude/skills/vercel-deployments/references/common-tasks.md";
const REL_CURSOR_SKILL: &str = ".cursor/skills/vercel-deployments/SKILL.md";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A summary of a Vercel project as returned by the list endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub team_id: Option<String>,
}

/// Verify-time per-id status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdStatus {
    /// Project exists on Vercel and matches the recorded ID.
    Ok,
    /// Project ID was not found on Vercel.
    Stale,
}

/// Trait abstracting Vercel MCP calls so tests can inject a mock.
pub trait VercelClient {
    fn list_projects(&self) -> anyhow::Result<Vec<ProjectSummary>>;
    fn get_project(&self, id: &str) -> anyhow::Result<Option<ProjectSummary>>;
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detection signal for Vercel usage in a repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VercelDetection {
    pub has_vercel_json: bool,
    pub has_at_vercel_dep: bool,
}

impl VercelDetection {
    pub fn detected(&self) -> bool {
        self.has_vercel_json || self.has_at_vercel_dep
    }
}

/// Detect Vercel usage in `repo_root` by checking for `vercel.json` at the
/// root and `@vercel/*` dependencies in `package.json`.
pub fn detect_vercel(repo_root: &Path) -> VercelDetection {
    let has_vercel_json = repo_root.join("vercel.json").is_file();
    let has_at_vercel_dep = repo_root
        .join("package.json")
        .is_file()
        .then(|| package_json_has_at_vercel_dep(&repo_root.join("package.json")))
        .unwrap_or(false);
    VercelDetection {
        has_vercel_json,
        has_at_vercel_dep,
    }
}

fn package_json_has_at_vercel_dep(package_json: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(package_json) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    for key in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(map) = parsed.get(key).and_then(|v| v.as_object()) {
            if map.keys().any(|k| k.starts_with("@vercel/")) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Fuzzy match
// ---------------------------------------------------------------------------

/// Result of matching the repo basename against a list of Vercel projects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchOutcome {
    /// Single high-confidence match — auto-confirm path.
    Strong(ProjectSummary),
    /// Multiple candidates within the match band — picker prompt path.
    Multiple(Vec<ProjectSummary>),
    /// No candidates within the match band.
    None,
}

/// Match `repo_basename` against `projects`. Strong = exactly one project
/// whose name equals or contains the repo basename; Multiple = several
/// candidates contain the basename; None = no match.
pub fn match_project(repo_basename: &str, projects: &[ProjectSummary]) -> MatchOutcome {
    let needle = repo_basename.to_ascii_lowercase();
    if needle.is_empty() {
        return MatchOutcome::None;
    }

    // Score: exact (case-insensitive) > contains > none.
    let mut exact: Vec<&ProjectSummary> = Vec::new();
    let mut contains: Vec<&ProjectSummary> = Vec::new();
    for p in projects {
        let n = p.name.to_ascii_lowercase();
        if n == needle {
            exact.push(p);
        } else if n.contains(&needle) || needle.contains(&n) {
            contains.push(p);
        }
    }

    if exact.len() == 1 {
        return MatchOutcome::Strong(exact[0].clone());
    }
    if exact.len() > 1 {
        return MatchOutcome::Multiple(exact.into_iter().cloned().collect());
    }
    match contains.len() {
        0 => MatchOutcome::None,
        1 => MatchOutcome::Strong(contains[0].clone()),
        _ => MatchOutcome::Multiple(contains.into_iter().cloned().collect()),
    }
}

// ---------------------------------------------------------------------------
// Template rendering
// ---------------------------------------------------------------------------

/// Render context: variables interpolated into the template.
#[derive(Debug, Clone)]
pub struct RenderContext {
    pub repo_name: String,
    pub projects: Vec<ProjectSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateTarget {
    Claude,
    Cursor,
}

/// Render the SKILL template with the given context. Produces a complete
/// markdown document ready to write or merge.
pub fn render_skill(ctx: &RenderContext, target: TemplateTarget) -> String {
    let raw = match target {
        TemplateTarget::Claude => TEMPLATE_CLAUDE_SKILL,
        TemplateTarget::Cursor => TEMPLATE_CURSOR_SKILL,
    };
    let ids_table = render_ids_table(&ctx.projects);
    let notes_placeholder = match target {
        TemplateTarget::Claude => {
            "- See `references/common-tasks.md` for typical workflows."
        }
        TemplateTarget::Cursor => {
            "- Add project-specific Vercel notes here (preserved across refresh)."
        }
    };
    raw.replace("{{REPO_NAME}}", &ctx.repo_name)
        .replace("{{IDS_TABLE}}", ids_table.trim_end())
        .replace("{{NOTES_PLACEHOLDER}}", notes_placeholder)
}

fn render_ids_table(projects: &[ProjectSummary]) -> String {
    if projects.is_empty() {
        return "_No Vercel projects captured. Run `cas integrate vercel refresh --update-ids` to fetch._".to_string();
    }
    let mut out = String::new();
    out.push_str("| Project | projectId | teamId |\n");
    out.push_str("|---------|-----------|--------|\n");
    for p in projects {
        out.push_str(&format!(
            "| {} | `{}` | `{}` |\n",
            p.name,
            p.id,
            p.team_id.as_deref().unwrap_or("-")
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Init / Refresh / Verify
// ---------------------------------------------------------------------------

/// Public CLI entry point. Routes to the correct sub-action.
///
/// Currently `Refresh` and `Verify` only accept the default behavior; the
/// `--update-ids` flag for refresh and id-level verbosity for verify are
/// surfaced by the lower-level `init/refresh_with_options/verify` calls used
/// directly by `cas init` once cas-7417 wires that path.
pub fn execute(action: IntegrationAction) -> anyhow::Result<IntegrationOutcome> {
    let repo_root = locate_repo_root()?;
    let client = make_default_client();
    match action {
        IntegrationAction::Init => init(&repo_root, client.as_ref()),
        IntegrationAction::Refresh => refresh(&repo_root, client.as_ref(), false),
        IntegrationAction::Verify => verify(&repo_root, client.as_ref()),
    }
}

fn locate_repo_root() -> anyhow::Result<PathBuf> {
    // Prefer git toplevel; fall back to current dir.
    let cwd = std::env::current_dir().context("getting current dir")?;
    if let Ok(out) = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }
    Ok(cwd)
}

/// `cas integrate vercel init`: detect, fetch, fuzzy-match, write 3 files.
pub fn init(
    repo_root: &Path,
    client: &dyn VercelClient,
) -> anyhow::Result<IntegrationOutcome> {
    let mut outcome = IntegrationOutcome::new(
        Platform::Vercel,
        IntegrationAction::Init,
        IntegrationStatus::Skipped,
    );

    let detection = detect_vercel(repo_root);
    if !detection.detected() {
        outcome
            .summary
            .push("vercel.json + @vercel/* deps not found; skipping".to_string());
        return Ok(outcome);
    }

    let claude_skill = repo_root.join(REL_CLAUDE_SKILL);
    if claude_skill.is_file() && !is_empty_or_placeholder(&claude_skill)? {
        outcome.status = IntegrationStatus::AlreadyConfigured;
        outcome.summary.push(format!(
            "{} already populated; use `cas integrate vercel refresh`",
            REL_CLAUDE_SKILL
        ));
        return Ok(outcome);
    }

    let projects = client
        .list_projects()
        .context("listing Vercel projects via MCP")?;

    let basename = repo_basename(repo_root);
    let chosen = match match_project(&basename, &projects) {
        MatchOutcome::Strong(p) => vec![p],
        MatchOutcome::Multiple(candidates) => candidates,
        MatchOutcome::None => {
            outcome.summary.push(format!(
                "no Vercel project matched repo name '{basename}' among {} projects; skipping",
                projects.len()
            ));
            return Ok(outcome);
        }
    };

    let ctx = RenderContext {
        repo_name: basename,
        projects: chosen.clone(),
    };
    let claude_doc = render_skill(&ctx, TemplateTarget::Claude);
    let cursor_doc = render_skill(&ctx, TemplateTarget::Cursor);

    write_file(&claude_skill, &claude_doc)?;
    write_file(&repo_root.join(REL_CLAUDE_REFS), TEMPLATE_COMMON_TASKS)?;
    write_file(&repo_root.join(REL_CURSOR_SKILL), &cursor_doc)?;

    outcome.status = IntegrationStatus::Configured;
    outcome.files = vec![
        PathBuf::from(REL_CLAUDE_SKILL),
        PathBuf::from(REL_CLAUDE_REFS),
        PathBuf::from(REL_CURSOR_SKILL),
    ];
    outcome.summary.push(format!(
        "captured {} Vercel project(s); wrote 3 files",
        chosen.len()
    ));
    Ok(outcome)
}

/// `cas integrate vercel refresh`: regenerate prose, preserve keep blocks.
/// When `update_ids` is true, re-fetch from Vercel and overwrite the
/// `<!-- keep vercel-ids -->` block.
pub fn refresh(
    repo_root: &Path,
    client: &dyn VercelClient,
    update_ids: bool,
) -> anyhow::Result<IntegrationOutcome> {
    let mut outcome = IntegrationOutcome::new(
        Platform::Vercel,
        IntegrationAction::Refresh,
        IntegrationStatus::Skipped,
    );

    let claude_path = repo_root.join(REL_CLAUDE_SKILL);
    let cursor_path = repo_root.join(REL_CURSOR_SKILL);

    if !claude_path.is_file() {
        outcome.summary.push(format!(
            "{} not found; run `cas integrate vercel init` first",
            REL_CLAUDE_SKILL
        ));
        return Ok(outcome);
    }

    // Determine projects to render: either freshly fetched (--update-ids) or
    // an empty list (the existing keep-block content survives via merge).
    let projects = if update_ids {
        let all = client
            .list_projects()
            .context("re-fetching Vercel projects via MCP")?;
        let basename = repo_basename(repo_root);
        match match_project(&basename, &all) {
            MatchOutcome::Strong(p) => vec![p],
            MatchOutcome::Multiple(c) => c,
            MatchOutcome::None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let basename = repo_basename(repo_root);
    let ctx = RenderContext {
        repo_name: basename,
        projects,
    };

    let mode = if update_ids {
        MergeMode::PreferTemplate
    } else {
        MergeMode::PreserveExisting
    };

    let claude_existing = std::fs::read_to_string(&claude_path).ok();
    let cursor_existing = std::fs::read_to_string(&cursor_path).ok();

    let new_claude = render_skill(&ctx, TemplateTarget::Claude);
    let new_cursor = render_skill(&ctx, TemplateTarget::Cursor);

    // Surface orphans before writing — only meaningful in PreserveExisting mode.
    if mode == MergeMode::PreserveExisting {
        if let Some(existing) = claude_existing.as_deref() {
            if let Ok(orphans) = keep_block::orphaned_existing(&new_claude, existing) {
                for o in orphans {
                    let label = o.name.unwrap_or_else(|| "<unnamed>".to_string());
                    outcome.summary.push(format!(
                        "warning: dropped hand-edited keep block '{label}' from {REL_CLAUDE_SKILL} (not present in current template)",
                    ));
                }
            }
        }
    }

    let merged_claude = keep_block::merge(&new_claude, claude_existing.as_deref(), mode)
        .context("merging claude SKILL.md")?;
    let merged_cursor = keep_block::merge(&new_cursor, cursor_existing.as_deref(), mode)
        .context("merging cursor SKILL.md")?;

    write_file(&claude_path, &merged_claude)?;
    write_file(&cursor_path, &merged_cursor)?;
    // common-tasks.md is regenerated unconditionally (no user content).
    write_file(&repo_root.join(REL_CLAUDE_REFS), TEMPLATE_COMMON_TASKS)?;

    outcome.status = IntegrationStatus::Refreshed;
    outcome.files = vec![
        PathBuf::from(REL_CLAUDE_SKILL),
        PathBuf::from(REL_CLAUDE_REFS),
        PathBuf::from(REL_CURSOR_SKILL),
    ];
    outcome.summary.push(if update_ids {
        "re-fetched Vercel IDs; refresh wrote 3 files".to_string()
    } else {
        "refreshed prose; keep blocks preserved".to_string()
    });
    Ok(outcome)
}

/// `cas integrate vercel verify`: parse keep block, ping each ID via MCP.
pub fn verify(
    repo_root: &Path,
    client: &dyn VercelClient,
) -> anyhow::Result<IntegrationOutcome> {
    let mut outcome = IntegrationOutcome::new(
        Platform::Vercel,
        IntegrationAction::Verify,
        IntegrationStatus::Skipped,
    );

    let claude_path = repo_root.join(REL_CLAUDE_SKILL);
    if !claude_path.is_file() {
        outcome.summary.push(format!(
            "{} not found; nothing to verify",
            REL_CLAUDE_SKILL
        ));
        return Ok(outcome);
    }

    let content = std::fs::read_to_string(&claude_path)
        .with_context(|| format!("reading {}", claude_path.display()))?;
    let blocks = keep_block::extract(&content)
        .context("parsing keep blocks in claude SKILL.md")?;
    let ids_block = blocks
        .into_iter()
        .find(|b| b.name.as_deref() == Some("vercel-ids"));
    let Some(ids_block) = ids_block else {
        outcome.summary.push(
            "no <!-- keep vercel-ids --> block found; nothing to verify".to_string(),
        );
        return Ok(outcome);
    };

    let recorded_ids = parse_recorded_ids(&ids_block.body);
    if recorded_ids.is_empty() {
        outcome
            .summary
            .push("no project IDs recorded in keep block".to_string());
        return Ok(outcome);
    }

    let mut statuses: BTreeMap<String, IdStatus> = BTreeMap::new();
    let mut any_stale = false;
    for id in &recorded_ids {
        let status = match client.get_project(id)? {
            Some(_) => IdStatus::Ok,
            None => {
                any_stale = true;
                IdStatus::Stale
            }
        };
        statuses.insert(id.clone(), status);
    }

    for (id, st) in &statuses {
        outcome.summary.push(format!("{id}: {:?}", st));
    }
    outcome.status = if any_stale {
        IntegrationStatus::Stale
    } else {
        IntegrationStatus::Configured
    };
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn repo_basename(repo_root: &Path) -> String {
    repo_root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string())
}

fn write_file(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, content)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn is_empty_or_placeholder(path: &Path) -> anyhow::Result<bool> {
    let s = std::fs::read_to_string(path)?;
    Ok(s.trim().is_empty())
}

/// Pull `prj_*` ids out of a recorded keep-block body. Tolerant of formatting
/// (table, list, prose) — extracts any backtick-fenced token starting with
/// `prj_`.
fn parse_recorded_ids(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_tick = false;
    for c in body.chars() {
        if c == '`' {
            if in_tick && !buf.is_empty() {
                if buf.starts_with("prj_") && !out.contains(&buf) {
                    out.push(buf.clone());
                }
                buf.clear();
            }
            in_tick = !in_tick;
            continue;
        }
        if in_tick {
            buf.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Default client factory (mcp-proxy gated)
// ---------------------------------------------------------------------------

fn make_default_client() -> Box<dyn VercelClient> {
    #[cfg(feature = "mcp-proxy")]
    {
        Box::new(mcp_proxy_client::ProxyVercelClient::new())
    }
    #[cfg(not(feature = "mcp-proxy"))]
    {
        Box::new(NoMcpVercelClient)
    }
}

#[cfg(not(feature = "mcp-proxy"))]
struct NoMcpVercelClient;

#[cfg(not(feature = "mcp-proxy"))]
impl VercelClient for NoMcpVercelClient {
    fn list_projects(&self) -> anyhow::Result<Vec<ProjectSummary>> {
        anyhow::bail!(
            "this build of cas does not include the mcp-proxy feature; rebuild with \
             `cargo install cas --features mcp-proxy` to enable `cas integrate vercel`"
        )
    }
    fn get_project(&self, _id: &str) -> anyhow::Result<Option<ProjectSummary>> {
        anyhow::bail!(
            "this build of cas does not include the mcp-proxy feature; rebuild with \
             `cargo install cas --features mcp-proxy` to enable `cas integrate vercel`"
        )
    }
}

#[cfg(feature = "mcp-proxy")]
mod mcp_proxy_client {
    //! Production [`VercelClient`] backed by the on-disk MCP proxy config
    //! (`.cas/proxy.toml` or `~/.config/code-mode-mcp/config.toml`).

    use super::{ProjectSummary, VercelClient};

    pub struct ProxyVercelClient;

    impl ProxyVercelClient {
        pub fn new() -> Self {
            Self
        }
    }

    impl VercelClient for ProxyVercelClient {
        fn list_projects(&self) -> anyhow::Result<Vec<ProjectSummary>> {
            // Wiring to cmcp_core::ProxyEngine requires a tokio runtime and
            // proxy config load; this is straightforward but involves the
            // proxy lifecycle. The trait separation here lets cas-7417
            // (init wire-up) supply a long-lived client built once at
            // startup. For now this returns a clear "not yet wired" error
            // so unit tests stay deterministic via the mock client and the
            // feature-gate path is exercised at compile time.
            anyhow::bail!(
                "ProxyVercelClient is not yet wired to cmcp_core::ProxyEngine; \
                 inject a custom VercelClient via init/refresh/verify directly \
                 (see vercel::init signature). Tracked separately."
            )
        }

        fn get_project(&self, _id: &str) -> anyhow::Result<Option<ProjectSummary>> {
            anyhow::bail!(
                "ProxyVercelClient is not yet wired to cmcp_core::ProxyEngine"
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::cell::RefCell;

    pub struct MockVercelClient {
        pub projects: Vec<ProjectSummary>,
        /// Project IDs that should be reported as "not found" by get_project.
        pub stale_ids: Vec<String>,
        /// Recorded calls for assertion.
        pub list_calls: RefCell<usize>,
        pub get_calls: RefCell<Vec<String>>,
    }

    impl MockVercelClient {
        pub fn new(projects: Vec<ProjectSummary>) -> Self {
            Self {
                projects,
                stale_ids: Vec::new(),
                list_calls: RefCell::new(0),
                get_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl VercelClient for MockVercelClient {
        fn list_projects(&self) -> anyhow::Result<Vec<ProjectSummary>> {
            *self.list_calls.borrow_mut() += 1;
            Ok(self.projects.clone())
        }
        fn get_project(&self, id: &str) -> anyhow::Result<Option<ProjectSummary>> {
            self.get_calls.borrow_mut().push(id.to_string());
            if self.stale_ids.iter().any(|s| s == id) {
                return Ok(None);
            }
            Ok(self.projects.iter().find(|p| p.id == id).cloned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MockVercelClient;
    use super::*;
    use tempfile::TempDir;

    fn proj(id: &str, name: &str, team: &str) -> ProjectSummary {
        ProjectSummary {
            id: id.to_string(),
            name: name.to_string(),
            team_id: Some(team.to_string()),
        }
    }

    fn make_repo(name: &str) -> TempDir {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        // Tempdir wrapper that exposes the named subdir? Just hand back tmp
        // and have callers pass `tmp.path().join(name)` as repo_root. Simpler:
        // callers use this directly.
        std::mem::forget(dir);
        tmp
    }

    fn make_repo_with_name(name: &str) -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        (tmp, root)
    }

    // --- Detection ------------------------------------------------------

    #[test]
    fn detect_vercel_finds_vercel_json() {
        let (_tmp, root) = make_repo_with_name("foo");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let d = detect_vercel(&root);
        assert!(d.has_vercel_json);
        assert!(!d.has_at_vercel_dep);
        assert!(d.detected());
    }

    #[test]
    fn detect_vercel_finds_at_vercel_dep_in_dependencies() {
        let (_tmp, root) = make_repo_with_name("foo");
        std::fs::write(
            root.join("package.json"),
            r#"{"dependencies":{"@vercel/og":"^0.5.0"}}"#,
        )
        .unwrap();
        let d = detect_vercel(&root);
        assert!(!d.has_vercel_json);
        assert!(d.has_at_vercel_dep);
        assert!(d.detected());
    }

    #[test]
    fn detect_vercel_finds_at_vercel_dep_in_devDependencies() {
        let (_tmp, root) = make_repo_with_name("foo");
        std::fs::write(
            root.join("package.json"),
            r#"{"devDependencies":{"@vercel/cli":"^33"}}"#,
        )
        .unwrap();
        assert!(detect_vercel(&root).detected());
    }

    #[test]
    fn detect_vercel_returns_false_when_neither_signal_present() {
        let (_tmp, root) = make_repo_with_name("foo");
        std::fs::write(root.join("package.json"), r#"{"dependencies":{}}"#).unwrap();
        assert!(!detect_vercel(&root).detected());
    }

    #[test]
    fn detect_vercel_ignores_unrelated_at_scoped_deps() {
        let (_tmp, root) = make_repo_with_name("foo");
        std::fs::write(
            root.join("package.json"),
            r#"{"dependencies":{"@types/node":"*"}}"#,
        )
        .unwrap();
        assert!(!detect_vercel(&root).detected());
    }

    // --- Fuzzy match ---------------------------------------------------

    #[test]
    fn match_project_strong_unique_exact() {
        let projects = vec![
            proj("prj_1", "myapp", "team_a"),
            proj("prj_2", "other", "team_a"),
        ];
        match match_project("myapp", &projects) {
            MatchOutcome::Strong(p) => assert_eq!(p.id, "prj_1"),
            other => panic!("expected Strong, got {other:?}"),
        }
    }

    #[test]
    fn match_project_strong_substring_unique() {
        let projects = vec![proj("prj_1", "myapp-frontend", "team_a")];
        match match_project("myapp", &projects) {
            MatchOutcome::Strong(p) => assert_eq!(p.id, "prj_1"),
            other => panic!("expected Strong, got {other:?}"),
        }
    }

    #[test]
    fn match_project_multiple_when_several_contain_basename() {
        let projects = vec![
            proj("prj_1", "myapp-frontend", "team_a"),
            proj("prj_2", "myapp-backend", "team_a"),
        ];
        match match_project("myapp", &projects) {
            MatchOutcome::Multiple(c) => assert_eq!(c.len(), 2),
            other => panic!("expected Multiple, got {other:?}"),
        }
    }

    #[test]
    fn match_project_none_when_no_overlap() {
        let projects = vec![proj("prj_1", "totally-unrelated", "team_a")];
        assert_eq!(match_project("myapp", &projects), MatchOutcome::None);
    }

    #[test]
    fn match_project_case_insensitive() {
        let projects = vec![proj("prj_1", "MyApp", "team_a")];
        match match_project("myapp", &projects) {
            MatchOutcome::Strong(p) => assert_eq!(p.id, "prj_1"),
            other => panic!("expected Strong, got {other:?}"),
        }
    }

    // --- Render ---------------------------------------------------------

    #[test]
    fn render_skill_claude_includes_named_keep_blocks() {
        let ctx = RenderContext {
            repo_name: "myapp".to_string(),
            projects: vec![proj("prj_1", "myapp", "team_a")],
        };
        let doc = render_skill(&ctx, TemplateTarget::Claude);
        assert!(doc.contains("<!-- keep vercel-ids -->"));
        assert!(doc.contains("<!-- /keep vercel-ids -->"));
        assert!(doc.contains("<!-- keep vercel-notes -->"));
        assert!(doc.contains("<!-- /keep vercel-notes -->"));
        assert!(doc.contains("prj_1"));
        assert!(doc.contains("team_a"));
        assert!(doc.contains("myapp"));
        assert!(!doc.contains("{{REPO_NAME}}"));
        assert!(!doc.contains("{{IDS_TABLE}}"));
        // Template must round-trip through the keep_block helper.
        keep_block::extract(&doc).expect("rendered template must be valid keep-block doc");
    }

    #[test]
    fn render_skill_cursor_is_single_file_variant() {
        let ctx = RenderContext {
            repo_name: "myapp".to_string(),
            projects: vec![proj("prj_1", "myapp", "team_a")],
        };
        let doc = render_skill(&ctx, TemplateTarget::Cursor);
        assert!(doc.contains("<!-- keep vercel-ids -->"));
        // Cursor variant references the cursor-side mcp skill path, not claude.
        assert!(doc.contains("~/.cursor/skills/mcp-vercel"));
        assert!(!doc.contains("references/common-tasks.md"));
    }

    // --- Init -----------------------------------------------------------

    #[test]
    fn init_writes_three_files_with_mock_client() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_1", "myapp", "team_a")]);

        let outcome = init(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Configured);
        assert_eq!(outcome.files.len(), 3);
        assert!(root.join(REL_CLAUDE_SKILL).is_file());
        assert!(root.join(REL_CLAUDE_REFS).is_file());
        assert!(root.join(REL_CURSOR_SKILL).is_file());

        let claude = std::fs::read_to_string(root.join(REL_CLAUDE_SKILL)).unwrap();
        assert!(claude.contains("prj_1"));
        assert!(claude.contains("<!-- keep vercel-ids -->"));
    }

    #[test]
    fn init_skipped_when_not_detected() {
        let (_tmp, root) = make_repo_with_name("myapp");
        let client = MockVercelClient::new(vec![]);
        let outcome = init(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Skipped);
        assert_eq!(*client.list_calls.borrow(), 0, "must not call MCP if not detected");
        assert!(!root.join(REL_CLAUDE_SKILL).exists());
    }

    #[test]
    fn init_skipped_when_no_match() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj(
            "prj_x",
            "totally-unrelated",
            "team_a",
        )]);
        let outcome = init(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Skipped);
        assert!(outcome
            .summary
            .iter()
            .any(|s| s.contains("no Vercel project matched")));
    }

    #[test]
    fn init_already_configured_when_skill_md_populated() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let path = root.join(REL_CLAUDE_SKILL);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# already here").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_1", "myapp", "team_a")]);

        let outcome = init(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::AlreadyConfigured);
        assert_eq!(*client.list_calls.borrow(), 0);
    }

    // --- Refresh --------------------------------------------------------

    #[test]
    fn refresh_skipped_when_skill_md_absent() {
        let (_tmp, root) = make_repo_with_name("myapp");
        let client = MockVercelClient::new(vec![]);
        let outcome = refresh(&root, &client, false).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Skipped);
    }

    #[test]
    fn refresh_preserves_keep_block_by_default() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_old", "myapp", "team_old")]);
        init(&root, &client).unwrap();

        // Hand-edit the keep-vercel-ids block.
        let path = root.join(REL_CLAUDE_SKILL);
        let original = std::fs::read_to_string(&path).unwrap();
        let edited = original.replace(
            "prj_old",
            "prj_USER_EDIT",
        );
        std::fs::write(&path, &edited).unwrap();

        // Refresh with a different upstream — but update_ids=false should
        // preserve the user's edit.
        let client2 = MockVercelClient::new(vec![proj("prj_new", "myapp", "team_new")]);
        let outcome = refresh(&root, &client2, false).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Refreshed);
        assert_eq!(*client2.list_calls.borrow(), 0, "default refresh must not hit MCP");

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("prj_USER_EDIT"), "keep block must survive: {after}");
        assert!(!after.contains("prj_new"));
    }

    #[test]
    fn refresh_with_update_ids_replaces_keep_block() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_old", "myapp", "team_old")]);
        init(&root, &client).unwrap();

        let client2 = MockVercelClient::new(vec![proj("prj_new", "myapp", "team_new")]);
        let outcome = refresh(&root, &client2, true).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Refreshed);
        assert!(*client2.list_calls.borrow() >= 1);

        let after = std::fs::read_to_string(root.join(REL_CLAUDE_SKILL)).unwrap();
        assert!(after.contains("prj_new"), "update-ids must rewrite IDs: {after}");
        assert!(!after.contains("prj_old"));
    }

    #[test]
    fn refresh_surfaces_orphaned_existing_keep_blocks() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_1", "myapp", "team_a")]);
        init(&root, &client).unwrap();

        // Inject an extra named keep block the template doesn't have.
        let path = root.join(REL_CLAUDE_SKILL);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push_str("\n<!-- keep vercel-extra -->\nuser hand-edited content\n<!-- /keep vercel-extra -->\n");
        std::fs::write(&path, &content).unwrap();

        let outcome = refresh(&root, &client, false).unwrap();
        assert!(
            outcome
                .summary
                .iter()
                .any(|s| s.contains("vercel-extra") && s.contains("dropped")),
            "must surface orphan: summary={:?}",
            outcome.summary
        );
    }

    // --- Verify ---------------------------------------------------------

    #[test]
    fn verify_skipped_when_skill_md_absent() {
        let (_tmp, root) = make_repo_with_name("myapp");
        let client = MockVercelClient::new(vec![]);
        let outcome = verify(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Skipped);
    }

    #[test]
    fn verify_classifies_ok_per_id_when_all_present() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_1", "myapp", "team_a")]);
        init(&root, &client).unwrap();

        let outcome = verify(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Configured);
        assert!(outcome.summary.iter().any(|s| s.contains("prj_1") && s.contains("Ok")));
    }

    #[test]
    fn verify_marks_stale_when_id_missing_from_mcp() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_1", "myapp", "team_a")]);
        init(&root, &client).unwrap();

        let mut stale_client =
            MockVercelClient::new(vec![proj("prj_1", "myapp", "team_a")]);
        stale_client.stale_ids.push("prj_1".to_string());
        let outcome = verify(&root, &stale_client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Stale);
        assert!(outcome.summary.iter().any(|s| s.contains("Stale")));
    }

    #[test]
    fn verify_returns_skipped_when_no_keep_block_present() {
        let (_tmp, root) = make_repo_with_name("myapp");
        let path = root.join(REL_CLAUDE_SKILL);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# no keep block here\n").unwrap();
        let client = MockVercelClient::new(vec![]);
        let outcome = verify(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Skipped);
    }

    // --- ID parsing -----------------------------------------------------

    #[test]
    fn parse_recorded_ids_extracts_prj_tokens() {
        let body = "| Project | projectId | teamId |\n|---|---|---|\n| a | `prj_abc` | `team_x` |\n| b | `prj_def` | `team_x` |";
        let ids = parse_recorded_ids(body);
        assert_eq!(ids, vec!["prj_abc".to_string(), "prj_def".to_string()]);
    }

    #[test]
    fn parse_recorded_ids_dedupes() {
        let body = "`prj_abc` and `prj_abc`";
        assert_eq!(parse_recorded_ids(body), vec!["prj_abc".to_string()]);
    }

    #[test]
    fn parse_recorded_ids_ignores_non_prj_tokens() {
        let body = "`team_x` and `prj_y`";
        assert_eq!(parse_recorded_ids(body), vec!["prj_y".to_string()]);
    }

    // Suppress unused warnings on the throwaway helper.
    #[allow(dead_code)]
    fn _unused() {
        let _ = make_repo("x");
    }
}
