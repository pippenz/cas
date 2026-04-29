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
//! at compile time via `include_str!`. They hold all project-specific IDs
//! inside named keep blocks (`<!-- keep vercel-ids -->`,
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

/// Detection sentinels (mirror this convention in cas-1ece / cas-f425).
const VERCEL_JSON: &str = "vercel.json";
const PACKAGE_JSON: &str = "package.json";
/// Vercel deploy ID prefix.
const PROJECT_ID_PREFIX: &str = "prj_";
/// Named keep block holding project-specific IDs.
const KEEP_IDS_BLOCK: &str = "vercel-ids";

/// Cap on bytes read from user-controlled markdown / JSON files.
/// Defends against `dd`-style accidents and symlinks to `/dev/zero`.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024; // 4 MiB

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
///
/// Both inputs are accessed via [`is_regular_file`] which rejects symlinks
/// and missing files; reads are size-capped at [`MAX_FILE_BYTES`].
pub fn detect_vercel(repo_root: &Path) -> VercelDetection {
    let has_vercel_json = is_regular_file(&repo_root.join(VERCEL_JSON));
    let has_at_vercel_dep =
        package_json_has_at_vercel_dep(&repo_root.join(PACKAGE_JSON));
    VercelDetection {
        has_vercel_json,
        has_at_vercel_dep,
    }
}

fn package_json_has_at_vercel_dep(package_json: &Path) -> bool {
    if !is_regular_file(package_json) {
        return false;
    }
    let Ok(content) = read_capped(package_json) else {
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

    // Score: exact (case-insensitive) > project-name contains repo-basename
    // > none. Note: we deliberately do NOT match when the project name is a
    // substring of the repo basename — short generic project names like
    // "app" or "web" would otherwise auto-match every repo. The project
    // name must contain the basename.
    let mut exact: Vec<&ProjectSummary> = Vec::new();
    let mut contains: Vec<&ProjectSummary> = Vec::new();
    for p in projects {
        let n = p.name.to_ascii_lowercase();
        if n == needle {
            exact.push(p);
        } else if n.contains(&needle) {
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
            escape_md_cell(&p.name),
            escape_md_cell_code(&p.id),
            escape_md_cell_code(p.team_id.as_deref().unwrap_or("-")),
        ));
    }
    out
}

/// Escape a value for safe inclusion in a markdown table cell. Strips:
/// - `|` (would break the cell layout)
/// - HTML comment open/close (`<!--`, `-->`) — would corrupt the surrounding
///   keep-block markers and on subsequent refresh cause `keep_block::extract`
///   to mis-parse.
/// - Newlines (would break the row).
fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|")
        .replace("<!--", "&lt;!--")
        .replace("-->", "--&gt;")
        .replace('\n', " ")
        .replace('\r', " ")
}

/// Escape a value rendered inside backticks (`code` cell). Same as
/// [`escape_md_cell`] but additionally strips backticks so a malicious id
/// cannot break out of the inline-code span and hijack the table.
fn escape_md_cell_code(s: &str) -> String {
    escape_md_cell(s).replace('`', "")
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
    let client = default_client();
    match action {
        IntegrationAction::Init => init(&repo_root, client.as_ref()),
        IntegrationAction::Refresh => refresh(&repo_root, client.as_ref(), false),
        IntegrationAction::Verify => verify(&repo_root, client.as_ref()),
    }
}

/// Sentinels that mark a directory as a real project root for the purposes
/// of `cas integrate`. If git toplevel resolution fails AND none of these
/// is present at the cwd, we refuse to write skill files into a bare CWD —
/// otherwise `cas integrate vercel init` from `~/Downloads` would silently
/// scribble `.claude/skills/...` into the user's home-adjacent directory.
const PROJECT_SENTINELS: &[&str] =
    &[".git", ".cas", "Cargo.toml", "package.json", "pyproject.toml"];

fn locate_repo_root() -> anyhow::Result<PathBuf> {
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
    // No git toplevel — only fall back to CWD if it looks like a project.
    if PROJECT_SENTINELS
        .iter()
        .any(|s| cwd.join(s).exists())
    {
        return Ok(cwd);
    }
    anyhow::bail!(
        "{} is not inside a project (no git toplevel, no .git/.cas/Cargo.toml/package.json/pyproject.toml). \
         Run `cas integrate vercel <action>` from a project root.",
        cwd.display()
    )
}

/// `cas integrate vercel init`: detect, fetch, fuzzy-match, write 3 files.
///
/// Convenience wrapper for [`init_with_preseed`] — equivalent to passing
/// `preseed_project_id = None`.
pub fn init(
    repo_root: &Path,
    client: &dyn VercelClient,
) -> anyhow::Result<IntegrationOutcome> {
    init_with_preseed(repo_root, client, None)
}

/// Same as [`init`] but accepts a pre-seeded `prj_*` id. When supplied, the
/// list-projects + fuzzy-match step is bypassed and the handler validates
/// the id via [`VercelClient::get_project`]. If the id is unknown to the
/// upstream MCP, returns `Skipped` with a warning rather than writing.
pub fn init_with_preseed(
    repo_root: &Path,
    client: &dyn VercelClient,
    preseed_project_id: Option<&str>,
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
    if claude_skill.is_file() && !is_empty(&claude_skill)? {
        outcome.status = IntegrationStatus::AlreadyConfigured;
        outcome.summary.push(format!(
            "{} already populated; use `cas integrate vercel refresh`",
            REL_CLAUDE_SKILL
        ));
        return Ok(outcome);
    }

    // Pre-seeded path: bypass list+match entirely, just validate via get.
    if let Some(preseed_id) = preseed_project_id {
        let basename = repo_basename(repo_root);
        let project = match client
            .get_project(preseed_id)
            .with_context(|| format!("validating pre-seeded vercel id {preseed_id}"))?
        {
            Some(p) => p,
            None => {
                outcome.summary.push(format!(
                    "vercel project '{preseed_id}' not found via MCP; skipping. \
                     Pass a different --vercel <id> or omit the flag to fall back to the picker."
                ));
                return Ok(outcome);
            }
        };
        let ctx = RenderContext {
            repo_name: basename,
            projects: vec![project],
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
        outcome
            .summary
            .push(format!("captured pre-seeded Vercel id {preseed_id}; wrote 3 files"));
        return Ok(outcome);
    }

    let projects = client
        .list_projects()
        .context("listing Vercel projects via MCP")?;

    let basename = repo_basename(repo_root);
    let chosen = match match_project(&basename, &projects) {
        MatchOutcome::Strong(p) => vec![p],
        MatchOutcome::Multiple(candidates) => {
            outcome.summary.push(format!(
                "captured {} ambiguous Vercel matches for '{basename}' (interactive picker pending cas-7417 wire-up)",
                candidates.len()
            ));
            candidates
        }
        MatchOutcome::None => {
            outcome.summary.push(format!(
                "no Vercel project matched repo name '{basename}' among {} projects; skipping",
                projects.len()
            ));
            return Ok(outcome);
        }
    };

    let chosen_len = chosen.len();
    let ctx = RenderContext {
        repo_name: basename,
        projects: chosen,
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
        "captured {chosen_len} Vercel project(s); wrote 3 files",
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

    let basename = repo_basename(repo_root);

    // Determine projects to render and merge mode:
    // - default refresh: empty `projects`; mode = PreserveExisting; existing
    //   keep-block content survives via merge.
    // - `--update-ids` with a real upstream match: fetched `projects`;
    //   mode = PreferTemplate; new IDs overwrite the keep block.
    // - `--update-ids` with NO upstream match: surface a warning and fall
    //   back to PreserveExisting so we never silently wipe recorded IDs.
    let (projects, mode) = if update_ids {
        let all = client
            .list_projects()
            .context("re-fetching Vercel projects via MCP")?;
        match match_project(&basename, &all) {
            MatchOutcome::Strong(p) => (vec![p], MergeMode::PreferTemplate),
            MatchOutcome::Multiple(c) => {
                outcome.summary.push(format!(
                    "captured {} ambiguous Vercel matches for '{basename}' (picker pending cas-7417)",
                    c.len()
                ));
                (c, MergeMode::PreferTemplate)
            }
            MatchOutcome::None => {
                outcome.summary.push(format!(
                    "warning: no Vercel project matched '{basename}' on refresh --update-ids; preserving existing keep-block IDs"
                ));
                (Vec::new(), MergeMode::PreserveExisting)
            }
        }
    } else {
        (Vec::new(), MergeMode::PreserveExisting)
    };

    let ctx = RenderContext {
        repo_name: basename,
        projects,
    };

    let claude_existing = read_capped(&claude_path).ok();
    let cursor_existing = read_capped(&cursor_path).ok();

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
        if let Some(existing) = cursor_existing.as_deref() {
            if let Ok(orphans) = keep_block::orphaned_existing(&new_cursor, existing) {
                for o in orphans {
                    let label = o.name.unwrap_or_else(|| "<unnamed>".to_string());
                    outcome.summary.push(format!(
                        "warning: dropped hand-edited keep block '{label}' from {REL_CURSOR_SKILL} (not present in current template)",
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

    let content = read_capped(&claude_path)
        .with_context(|| format!("reading {}", claude_path.display()))?;
    let blocks = keep_block::extract(&content)
        .context("parsing keep blocks in claude SKILL.md")?;
    let ids_block = blocks
        .into_iter()
        .find(|b| b.name.as_deref() == Some(KEEP_IDS_BLOCK));
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

/// Write `content` to `path`, creating parent directories as needed.
///
/// **Not atomic.** A process kill between the parent-dir create and the file
/// write, or mid-write, can leave a partial file on disk. Init's three-file
/// write sequence inherits this — partial-state recovery is tracked in
/// **cas-7417** (init wire-up) which already needs to thread a long-lived
/// client + tokio runtime and is the natural place for tempfile-and-rename.
fn write_file(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, content)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Treat a SKILL.md as "ready to overwrite" if it does not exist or its
/// trimmed contents are empty. Used by `init` to refuse overwriting populated
/// SKILL files. We deliberately stay conservative — partial-state recovery is
/// out of scope for this task and tracked separately.
fn is_empty(path: &Path) -> anyhow::Result<bool> {
    let s = read_capped(path)?;
    Ok(s.trim().is_empty())
}

/// Pull `prj_*` ids out of a recorded keep-block body. Tolerant of
/// formatting (table, list, prose, fenced code) — extracts any token of the
/// form `prj_<word>` regardless of surrounding markdown punctuation.
fn parse_recorded_ids(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut iter = body.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if c == 'p' && body[i..].starts_with(PROJECT_ID_PREFIX) {
            // Capture the longest run of [A-Za-z0-9_] starting at i.
            let mut end = i + PROJECT_ID_PREFIX.len();
            for (j, cc) in body[end..].char_indices() {
                if cc.is_ascii_alphanumeric() || cc == '_' {
                    end = i + PROJECT_ID_PREFIX.len() + j + cc.len_utf8();
                } else {
                    break;
                }
            }
            let token = body[i..end].to_string();
            if token.len() > PROJECT_ID_PREFIX.len() && !out.contains(&token) {
                out.push(token);
            }
            // Skip iter forward to the end of the captured token.
            while let Some(&(k, _)) = iter.peek() {
                if k < end {
                    iter.next();
                } else {
                    break;
                }
            }
        }
    }
    out
}

/// True iff `path` exists, is a regular file, and is NOT a symlink.
fn is_regular_file(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(md) => md.file_type().is_file(),
        Err(_) => false,
    }
}

/// Read a file with a [`MAX_FILE_BYTES`] cap. Refuses symlinks and rejects
/// inputs larger than the cap with a clear error rather than allocating
/// unbounded memory.
fn read_capped(path: &Path) -> anyhow::Result<String> {
    use std::io::Read;
    let md = std::fs::symlink_metadata(path)
        .with_context(|| format!("statting {}", path.display()))?;
    if md.file_type().is_symlink() {
        anyhow::bail!(
            "{} is a symlink; refusing to follow",
            path.display()
        );
    }
    if md.len() > MAX_FILE_BYTES {
        anyhow::bail!(
            "{} is {} bytes; exceeds cap of {} bytes",
            path.display(),
            md.len(),
            MAX_FILE_BYTES
        );
    }
    let mut f = std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let mut s = String::new();
    f.take(MAX_FILE_BYTES + 1).read_to_string(&mut s)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(s)
}

// ---------------------------------------------------------------------------
// Default client factory (mcp-proxy gated)
// ---------------------------------------------------------------------------

/// Production [`VercelClient`] factory. Exposed for the orchestration layer
/// (`integrations::run`) so it can construct a client per `cas init`
/// invocation. Tests should NOT use this — inject a `MockVercelClient`
/// directly.
pub fn default_client() -> Box<dyn VercelClient> {
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
    //!
    //! Wires `mcp__vercel__list_projects` and `mcp__vercel__get_project`
    //! through `cmcp_core::ProxyEngine`. A tokio current-thread runtime is
    //! spun up per call (the CLI is otherwise sync) — fine for the
    //! handful of calls one `cas integrate vercel <action>` makes.
    //!
    //! **Live MCP is not exercised by `cargo test`** — the parsers in
    //! [`super::parse_list_projects`] / [`super::parse_get_project`] are
    //! fixture-tested against canonical Vercel MCP response shapes. The
    //! transport itself is verified manually.

    use anyhow::Context;
    use serde_json::Value;

    use super::{parse_get_project, parse_list_projects, ProjectSummary, VercelClient};

    pub(super) struct ProxyVercelClient;

    impl ProxyVercelClient {
        pub(super) fn new() -> Self {
            Self
        }

        /// Resolve a proxy config path: first `<cas_root>/proxy.toml` if cas
        /// is initialized, else the user-level fallback the cmcp_core loader
        /// already handles.
        fn proxy_config_path() -> Option<std::path::PathBuf> {
            crate::store::find_cas_root()
                .ok()
                .map(|r| r.join("proxy.toml"))
                .filter(|p| p.exists())
        }

        async fn call(
            tool: &str,
            args: Option<serde_json::Map<String, Value>>,
        ) -> anyhow::Result<Value> {
            let cfg = cmcp_core::config::Config::load_merged(
                Self::proxy_config_path().as_deref(),
            )
            .context("loading MCP proxy config")?;
            anyhow::ensure!(
                !cfg.servers.is_empty(),
                "no MCP servers configured. Run `cas mcp add vercel ...` or check ~/.config/code-mode-mcp/config.toml."
            );
            let engine = cmcp_core::ProxyEngine::from_configs(cfg.servers)
                .await
                .context("starting MCP proxy engine")?;
            // Run the call; capture Result so we can shut down the engine on
            // both the success and error paths. Otherwise a transport
            // failure would leak the spawned upstream MCP child process for
            // the lifetime of `cas init`.
            let outcome = engine
                .call_tool("vercel", tool, args)
                .await
                .with_context(|| format!("calling vercel.{tool}"));
            engine.shutdown().await;
            outcome
        }

        fn block_on<F, T>(fut: F) -> anyhow::Result<T>
        where
            F: std::future::Future<Output = anyhow::Result<T>>,
        {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("building tokio runtime")?;
            rt.block_on(fut)
        }
    }

    impl VercelClient for ProxyVercelClient {
        fn list_projects(&self) -> anyhow::Result<Vec<ProjectSummary>> {
            let value = Self::block_on(Self::call("list_projects", None))?;
            parse_list_projects(&value)
        }

        fn get_project(&self, id: &str) -> anyhow::Result<Option<ProjectSummary>> {
            let mut args = serde_json::Map::new();
            args.insert("projectId".to_string(), Value::String(id.to_string()));
            let value = Self::block_on(Self::call("get_project", Some(args)))?;
            parse_get_project(&value)
        }
    }
}

// ---------------------------------------------------------------------------
// MCP response parsers (fixture-testable, transport-independent).
// ---------------------------------------------------------------------------

/// Parse the response from `mcp__vercel__list_projects` into a vec of
/// [`ProjectSummary`]. Tolerant of:
///
/// - The raw RPC envelope (`{ content: [{ type: "text", text: "<json>" }] }`)
///   that `cmcp_core::ProxyEngine::call_tool` returns.
/// - A bare JSON array of project objects.
/// - A top-level object with a `projects` field.
pub fn parse_list_projects(value: &serde_json::Value) -> anyhow::Result<Vec<ProjectSummary>> {
    use serde_json::Value;
    let inner = unwrap_mcp_envelope(value)?;
    let array = match &inner {
        Value::Array(a) => a.clone(),
        Value::Object(map) => map
            .get("projects")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => anyhow::bail!("unexpected list_projects shape: {inner}"),
    };
    let mut out = Vec::with_capacity(array.len());
    for v in array {
        if let Some(p) = project_from_value(&v) {
            out.push(p);
        }
    }
    Ok(out)
}

/// Parse the response from `mcp__vercel__get_project` into an
/// `Option<ProjectSummary>`. Returns `None` only for explicit not-found
/// signals (`null`, empty content, `{ error: "<text containing 'not found'>" }`).
/// Real transport / MCP failures (`isError: true` envelopes, JSON parse
/// errors) are propagated as `Err`, NOT silently swallowed into None — a
/// stale-id verifier must never confuse "MCP unreachable" with "project
/// genuinely missing".
pub fn parse_get_project(value: &serde_json::Value) -> anyhow::Result<Option<ProjectSummary>> {
    use serde_json::Value;
    let inner = unwrap_mcp_envelope(value)?;
    if matches!(&inner, Value::Null) {
        return Ok(None);
    }
    if let Value::Object(map) = &inner {
        if let Some(err) = map.get("error").and_then(|v| v.as_str()) {
            if err.to_ascii_lowercase().contains("not found") {
                return Ok(None);
            }
        }
    }
    Ok(project_from_value(&inner))
}

/// MCP tool calls return an envelope of the form
/// `{ content: [{ type: "text", text: "<json string>" }, ...], isError: bool }`.
/// Strip that wrapper and parse the inner JSON. If `value` is not the
/// envelope shape, return it unchanged.
fn unwrap_mcp_envelope(value: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    use serde_json::Value;
    let Value::Object(map) = value else {
        return Ok(value.clone());
    };
    if map.get("isError").and_then(|v| v.as_bool()) == Some(true) {
        anyhow::bail!("MCP returned isError=true: {value}");
    }
    let Some(Value::Array(content)) = map.get("content") else {
        return Ok(value.clone());
    };
    // Concatenate any "text" parts then parse as JSON.
    let mut buf = String::new();
    for item in content {
        if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
            buf.push_str(t);
        }
    }
    if buf.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&buf).with_context(|| format!("parsing MCP text content: {buf}"))
}

fn project_from_value(v: &serde_json::Value) -> Option<ProjectSummary> {
    let id = v.get("id")?.as_str()?.to_string();
    let name = v.get("name")?.as_str()?.to_string();
    // Vercel returns `accountId` (the team/user owning the project) in the
    // standard list shape. Some envelopes use `teamId` directly.
    let team_id = v
        .get("teamId")
        .and_then(|s| s.as_str())
        .or_else(|| v.get("accountId").and_then(|s| s.as_str()))
        .map(|s| s.to_string());
    Some(ProjectSummary { id, name, team_id })
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
    fn detect_vercel_finds_at_vercel_dep_in_dev_dependencies() {
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

    #[test]
    fn parse_recorded_ids_extracts_from_unfenced_prose() {
        // No backticks at all — still extract the prj_ token.
        let body = "Production project is prj_abc123 on team team_x.";
        assert_eq!(parse_recorded_ids(body), vec!["prj_abc123".to_string()]);
    }

    #[test]
    fn parse_recorded_ids_extracts_from_triple_backtick_fence() {
        let body = "```\nconst id = \"prj_fenced\";\n```";
        assert_eq!(parse_recorded_ids(body), vec!["prj_fenced".to_string()]);
    }

    #[test]
    fn parse_recorded_ids_returns_empty_for_empty_or_no_match() {
        assert!(parse_recorded_ids("").is_empty());
        assert!(parse_recorded_ids("no ids here").is_empty());
    }

    // --- Detection edge cases ------------------------------------------

    #[test]
    fn detect_vercel_finds_at_vercel_dep_in_peer_dependencies() {
        let (_tmp, root) = make_repo_with_name("foo");
        std::fs::write(
            root.join("package.json"),
            r#"{"peerDependencies":{"@vercel/og":"*"}}"#,
        )
        .unwrap();
        assert!(detect_vercel(&root).detected());
    }

    #[test]
    fn detect_vercel_rejects_symlinked_package_json() {
        let (_tmp, root) = make_repo_with_name("foo");
        let real = root.join("real.json");
        std::fs::write(&real, r#"{"dependencies":{"@vercel/og":"*"}}"#).unwrap();
        let link = root.join("package.json");
        // Best-effort symlink; skip on platforms without symlink support.
        if std::os::unix::fs::symlink(&real, &link).is_err() {
            return;
        }
        let d = detect_vercel(&root);
        assert!(
            !d.has_at_vercel_dep,
            "symlinked package.json must not be followed"
        );
    }

    #[test]
    fn detect_vercel_handles_malformed_package_json() {
        let (_tmp, root) = make_repo_with_name("foo");
        std::fs::write(root.join("package.json"), "this is not json").unwrap();
        // Should not panic and should not crash; just returns false.
        assert!(!detect_vercel(&root).has_at_vercel_dep);
    }

    // --- match_project: short generic name guard -----------------------

    #[test]
    fn match_project_does_not_match_short_generic_project_into_long_basename() {
        // Pre-fix this would match: "myapp-web".contains("app") was true.
        let projects = vec![proj("prj_x", "app", "team_a")];
        assert_eq!(match_project("myapp-web", &projects), MatchOutcome::None);
    }

    // --- Render escaping ------------------------------------------------

    #[test]
    fn render_skill_escapes_pipe_and_html_comment_in_project_name() {
        let ctx = RenderContext {
            repo_name: "myapp".to_string(),
            projects: vec![proj(
                "prj_1",
                "evil | name <!-- /keep vercel-ids -->",
                "team_a",
            )],
        };
        let doc = render_skill(&ctx, TemplateTarget::Claude);
        // Must remain valid keep-block document — no premature close.
        keep_block::extract(&doc).expect("escaped doc must round-trip");
        assert!(!doc.contains("<!-- /keep vercel-ids --> bar"));
    }

    #[test]
    fn render_skill_with_empty_projects_emits_placeholder() {
        let ctx = RenderContext {
            repo_name: "myapp".to_string(),
            projects: Vec::new(),
        };
        let doc = render_skill(&ctx, TemplateTarget::Claude);
        assert!(doc.contains("No Vercel projects captured"));
        keep_block::extract(&doc).expect("empty-projects doc must round-trip");
    }

    // --- Init: multi-match ---------------------------------------------

    #[test]
    fn init_writes_all_candidates_when_match_is_ambiguous() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![
            proj("prj_fe", "myapp-frontend", "team_a"),
            proj("prj_be", "myapp-backend", "team_a"),
        ]);
        let outcome = init(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Configured);
        let claude = std::fs::read_to_string(root.join(REL_CLAUDE_SKILL)).unwrap();
        assert!(claude.contains("prj_fe"));
        assert!(claude.contains("prj_be"));
        assert!(outcome.summary.iter().any(|s| s.contains("ambiguous")));
    }

    // --- Init: empty SKILL.md is treated as fresh ----------------------

    #[test]
    fn init_treats_empty_existing_skill_md_as_fresh() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let path = root.join(REL_CLAUDE_SKILL);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "  \n  \n").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_1", "myapp", "team_a")]);

        let outcome = init(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Configured);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("prj_1"));
    }

    // --- Refresh: cursor-side orphan detection -------------------------

    #[test]
    fn refresh_surfaces_orphaned_keep_blocks_in_cursor_skill_too() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_1", "myapp", "team_a")]);
        init(&root, &client).unwrap();

        let cursor_path = root.join(REL_CURSOR_SKILL);
        let mut cursor = std::fs::read_to_string(&cursor_path).unwrap();
        cursor.push_str(
            "\n<!-- keep vercel-cursor-extra -->\nuser content\n<!-- /keep vercel-cursor-extra -->\n",
        );
        std::fs::write(&cursor_path, &cursor).unwrap();

        let outcome = refresh(&root, &client, false).unwrap();
        assert!(
            outcome
                .summary
                .iter()
                .any(|s| s.contains("vercel-cursor-extra") && s.contains(REL_CURSOR_SKILL)),
            "cursor orphan must be surfaced: summary={:?}",
            outcome.summary
        );
    }

    // --- Refresh: --update-ids with no upstream match falls back -------

    #[test]
    fn refresh_with_update_ids_and_no_match_preserves_existing() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_old", "myapp", "team_old")]);
        init(&root, &client).unwrap();

        // Upstream now has nothing matching the basename.
        let stranger = MockVercelClient::new(vec![proj(
            "prj_other",
            "totally-unrelated",
            "team_b",
        )]);
        let outcome = refresh(&root, &stranger, true).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Refreshed);
        assert!(
            outcome
                .summary
                .iter()
                .any(|s| s.contains("preserving existing")),
            "must surface fallback warning: summary={:?}",
            outcome.summary
        );
        // IDs from init survive.
        let after = std::fs::read_to_string(root.join(REL_CLAUDE_SKILL)).unwrap();
        assert!(after.contains("prj_old"));
        assert!(!after.contains("prj_other"));
    }

    // --- Verify: mixed-staleness + empty ID block ----------------------

    #[test]
    fn verify_classifies_mixed_ok_and_stale_per_id() {
        let (_tmp, root) = make_repo_with_name("myapp");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![
            proj("prj_a", "myapp-frontend", "team_a"),
            proj("prj_b", "myapp-backend", "team_a"),
        ]);
        init(&root, &client).unwrap();

        let mut mixed = MockVercelClient::new(vec![
            proj("prj_a", "myapp-frontend", "team_a"),
            proj("prj_b", "myapp-backend", "team_a"),
        ]);
        mixed.stale_ids.push("prj_a".to_string());
        let outcome = verify(&root, &mixed).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Stale);
        assert!(outcome.summary.iter().any(|s| s.contains("prj_a") && s.contains("Stale")));
        assert!(outcome.summary.iter().any(|s| s.contains("prj_b") && s.contains("Ok")));
    }

    #[test]
    fn verify_returns_skipped_when_keep_block_has_no_ids() {
        let (_tmp, root) = make_repo_with_name("myapp");
        let path = root.join(REL_CLAUDE_SKILL);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "# vercel\n<!-- keep vercel-ids -->\n_no ids yet_\n<!-- /keep vercel-ids -->\n",
        )
        .unwrap();
        let client = MockVercelClient::new(vec![]);
        let outcome = verify(&root, &client).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Skipped);
        assert!(outcome.summary.iter().any(|s| s.contains("no project IDs recorded")));
    }

    // --- read_capped: symlink + size guards -----------------------------

    #[test]
    fn read_capped_rejects_symlinks() {
        let (_tmp, root) = make_repo_with_name("foo");
        let real = root.join("real.txt");
        std::fs::write(&real, "hi").unwrap();
        let link = root.join("link.txt");
        if std::os::unix::fs::symlink(&real, &link).is_err() {
            return;
        }
        let err = read_capped(&link).unwrap_err().to_string();
        assert!(err.contains("symlink"), "got: {err}");
    }

    // --- MCP response parsers ------------------------------------------

    #[test]
    fn parse_list_projects_handles_mcp_text_envelope() {
        let v = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "[{\"id\":\"prj_1\",\"name\":\"myapp\",\"accountId\":\"team_a\"}]"
            }]
        });
        let projects = parse_list_projects(&v).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "prj_1");
        assert_eq!(projects[0].name, "myapp");
        assert_eq!(projects[0].team_id.as_deref(), Some("team_a"));
    }

    #[test]
    fn parse_list_projects_handles_bare_array() {
        let v = serde_json::json!([
            {"id":"prj_1","name":"myapp","teamId":"team_a"},
            {"id":"prj_2","name":"other","accountId":"team_b"},
        ]);
        let projects = parse_list_projects(&v).unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[1].team_id.as_deref(), Some("team_b"));
    }

    #[test]
    fn parse_list_projects_handles_object_with_projects_field() {
        let v = serde_json::json!({
            "projects": [
                {"id":"prj_1","name":"myapp","teamId":"team_a"},
            ],
            "pagination": {}
        });
        let projects = parse_list_projects(&v).unwrap();
        assert_eq!(projects.len(), 1);
    }

    #[test]
    fn parse_list_projects_skips_malformed_entries_without_panicking() {
        let v = serde_json::json!([
            {"id":"prj_1","name":"good","teamId":"team_a"},
            {"id":"prj_2"},                 // missing name
            "not an object",
            {"id":"prj_3","name":"good3","teamId":"team_a"}
        ]);
        let projects = parse_list_projects(&v).unwrap();
        // Only the well-formed entries land.
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].id, "prj_1");
        assert_eq!(projects[1].id, "prj_3");
    }

    #[test]
    fn parse_list_projects_propagates_mcp_error_envelope() {
        let v = serde_json::json!({
            "isError": true,
            "content": [{"type":"text","text":"boom"}]
        });
        assert!(parse_list_projects(&v).is_err());
    }

    #[test]
    fn parse_get_project_returns_some_for_valid_envelope() {
        let v = serde_json::json!({
            "content": [{"type":"text","text":"{\"id\":\"prj_1\",\"name\":\"myapp\",\"teamId\":\"team_a\"}"}]
        });
        let p = parse_get_project(&v).unwrap();
        assert_eq!(p.unwrap().id, "prj_1");
    }

    #[test]
    fn parse_get_project_returns_none_for_null_or_not_found() {
        let null_v = serde_json::json!(null);
        assert!(parse_get_project(&null_v).unwrap().is_none());
        let not_found = serde_json::json!({"error":"project not found"});
        assert!(parse_get_project(&not_found).unwrap().is_none());
        let empty_envelope =
            serde_json::json!({"content":[{"type":"text","text":""}]});
        assert!(parse_get_project(&empty_envelope).unwrap().is_none());
    }

    #[test]
    fn parse_get_project_propagates_is_error_envelope_as_err() {
        // A real MCP transport / auth failure must NOT silently collapse
        // into Ok(None) — that would let verify report "stale" for projects
        // the caller can't actually reach.
        let v = serde_json::json!({
            "isError": true,
            "content": [{"type":"text","text":"auth failure"}]
        });
        assert!(parse_get_project(&v).is_err());
    }

    // --- locate_repo_root sentinel -------------------------------------

    #[test]
    fn locate_repo_root_errors_outside_a_project() {
        // CWD is a fresh tmp dir with no markers. locate_repo_root() must
        // refuse to fall back to it. We cd via std::env::set_current_dir;
        // restore in a guard.
        let tmp = TempDir::new().unwrap();
        let prev = std::env::current_dir().unwrap();
        struct Guard(std::path::PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }
        let _g = Guard(prev);
        std::env::set_current_dir(tmp.path()).unwrap();
        // This test is racy across other tests that also set_current_dir;
        // best-effort. If the helper happens to find a git repo above the
        // tempdir, skip — only assert that absent any sentinel + no git
        // repo, the fn errors. We can't easily induce that condition
        // reliably across CI environments, so the test is best-effort
        // and only fails if locate_repo_root returns the bare tmp path
        // (i.e., the silent-fallback bug is present).
        if let Ok(p) = locate_repo_root() {
            // Acceptable only if the resolved path is NOT the bare tmp dir.
            assert_ne!(
                p.canonicalize().unwrap_or(p.clone()),
                tmp.path().canonicalize().unwrap()
            );
        }
    }

    #[test]
    fn locate_repo_root_accepts_dir_with_cargo_toml_sentinel() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "[package]\nname=\"x\"\nversion=\"0.1\"\nedition=\"2024\"\n").unwrap();
        let prev = std::env::current_dir().unwrap();
        struct Guard(std::path::PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }
        let _g = Guard(prev);
        std::env::set_current_dir(tmp.path()).unwrap();
        // Either git toplevel returns a sane path or sentinel fallback
        // accepts the cwd. Both are valid; we just require Ok.
        let _ = locate_repo_root().unwrap();
    }

    // --- Preseed init path ---------------------------------------------

    #[test]
    fn init_with_preseed_writes_files_when_id_resolves() {
        let (_tmp, root) = make_repo_with_name("anything");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let client = MockVercelClient::new(vec![proj("prj_seed", "wholly-different-name", "team_a")]);

        let outcome = init_with_preseed(&root, &client, Some("prj_seed")).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Configured);
        let claude = std::fs::read_to_string(root.join(REL_CLAUDE_SKILL)).unwrap();
        assert!(claude.contains("prj_seed"));
        // Pre-seed bypasses list_projects entirely.
        assert_eq!(*client.list_calls.borrow(), 0);
        // But it does call get_project to validate.
        assert!(client
            .get_calls
            .borrow()
            .iter()
            .any(|c| c == "prj_seed"));
    }

    #[test]
    fn init_with_preseed_skips_when_id_not_found() {
        let (_tmp, root) = make_repo_with_name("anything");
        std::fs::write(root.join("vercel.json"), "{}").unwrap();
        let mut client = MockVercelClient::new(vec![]);
        client.stale_ids.push("prj_unknown".to_string());

        let outcome =
            init_with_preseed(&root, &client, Some("prj_unknown")).unwrap();
        assert_eq!(outcome.status, IntegrationStatus::Skipped);
        assert!(outcome
            .summary
            .iter()
            .any(|s| s.contains("not found via MCP")));
    }

    #[test]
    fn read_capped_rejects_oversize_files() {
        let (_tmp, root) = make_repo_with_name("foo");
        let path = root.join("big.txt");
        let big = vec![b'a'; (MAX_FILE_BYTES as usize) + 1];
        std::fs::write(&path, &big).unwrap();
        let err = read_capped(&path).unwrap_err().to_string();
        assert!(err.contains("exceeds cap"), "got: {err}");
    }
}
