//! Structural guardrails for the `cas-code-review` orchestrator SKILL.md
//! (Phase 1 Subsystem A, Unit 4 — task cas-71ed).
//!
//! The skill content itself is a prompt and therefore not unit-testable in
//! any meaningful semantic way. These tests instead pin the *structure* so
//! future edits cannot silently drop a section, forget a mirror, or
//! regress the "LLM-judged, not path pattern matching" posture the
//! brainstorm requires.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR here is `<workspace>/crates/cas-store` — two
    // parents up is the workspace root, where the SKILL.md mirrors
    // this test checks actually live.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn claude_skill_path() -> PathBuf {
    repo_root().join("cas-cli/src/builtins/skills/cas-code-review/SKILL.md")
}

fn codex_skill_path() -> PathBuf {
    repo_root().join("cas-cli/src/builtins/codex/skills/cas-code-review/SKILL.md")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Required headings (as substring matches against lines starting with
/// `##` or `###`). Order is not asserted — only presence — because edits
/// that reorder sections should not fail the test. These headings reflect
/// the 8 sections called out in the task description.
const REQUIRED_SECTIONS: &[&str] = &[
    "Purpose",
    "Inputs",
    "Step 1", // Intent extraction
    "Step 2", // Conditional persona selection
    "Step 3", // Parallel dispatch
    "Step 4", // Hand off to merge (Unit 5)
    "Step 5", // Mode-specific output
    "Mode reference",
];

fn assert_sections_present(skill: &str, label: &str) {
    for section in REQUIRED_SECTIONS {
        let found = skill
            .lines()
            .filter(|l| l.trim_start().starts_with('#'))
            .any(|l| l.contains(section));
        assert!(
            found,
            "[{label}] missing required section heading containing '{section}'"
        );
    }
}

#[test]
fn claude_mirror_skill_exists_and_is_structurally_complete() {
    let path = claude_skill_path();
    assert!(
        path.exists(),
        "claude mirror SKILL.md missing at {}",
        path.display()
    );
    let body = read(&path);

    // Frontmatter must declare the skill name and the managed_by marker
    // so `cas sync` treats it as an authoritative file.
    assert!(
        body.starts_with("---\n"),
        "claude SKILL.md must start with YAML frontmatter"
    );
    assert!(
        body.contains("name: cas-code-review"),
        "claude SKILL.md frontmatter must set name: cas-code-review"
    );
    assert!(
        body.contains("managed_by: cas"),
        "claude SKILL.md frontmatter must set managed_by: cas"
    );

    assert_sections_present(&body, "claude");
}

#[test]
fn codex_mirror_skill_exists_and_is_structurally_complete() {
    let path = codex_skill_path();
    assert!(
        path.exists(),
        "codex mirror SKILL.md missing at {}",
        path.display()
    );
    let body = read(&path);

    assert!(
        body.starts_with("---\n"),
        "codex SKILL.md must start with YAML frontmatter"
    );
    assert!(
        body.contains("name: cas-code-review"),
        "codex SKILL.md frontmatter must set name: cas-code-review"
    );
    assert!(
        body.contains("managed_by: cas"),
        "codex SKILL.md frontmatter must set managed_by: cas"
    );

    assert_sections_present(&body, "codex");
}

#[test]
fn both_mirrors_are_content_equivalent() {
    let a = read(&claude_skill_path());
    let b = read(&codex_skill_path());
    assert_eq!(
        a, b,
        "claude and codex SKILL.md mirrors must be byte-identical so \
         `cas sync` doesn't oscillate between them"
    );
}

#[test]
fn persona_activation_is_explicitly_llm_judged_not_pattern_matched() {
    // Defensive guardrail per task description: future edits must not
    // regress to "grep the diff for /auth/ and activate security" —
    // that's exactly the drift R2 warns against. Require the phrase
    // to appear verbatim so a drive-by edit can't quietly remove it.
    let body = read(&claude_skill_path());
    assert!(
        body.contains("LLM-judged, not path pattern matching"),
        "SKILL.md must retain the verbatim anti-drift phrase \
         'LLM-judged, not path pattern matching' in the persona \
         activation section"
    );
}

#[test]
fn mode_reference_table_renders() {
    // R8 requires a reference table covering all four invocation modes.
    // The table is a standard pipe-table; we don't pin the columns, but
    // we do pin that all four mode names appear inside a row-looking
    // line (starts with `|`).
    let body = read(&claude_skill_path());
    let modes = ["autofix", "interactive", "report-only", "headless"];
    for mode in modes {
        let row_hit = body
            .lines()
            .filter(|l| l.trim_start().starts_with('|'))
            .any(|l| l.contains(mode));
        assert!(
            row_hit,
            "mode reference table must contain a row mentioning '{mode}'"
        );
    }
}

#[test]
fn findings_schema_reference_is_wired_up() {
    // Step 3 dispatches personas with "findings-schema reference" —
    // the orchestrator should point at the canonical doc so personas
    // can load it. Pin that the relative path appears somewhere in
    // the skill body.
    let body = read(&claude_skill_path());
    assert!(
        body.contains("references/findings-schema.md"),
        "SKILL.md must reference references/findings-schema.md so \
         dispatched personas know where the envelope contract lives"
    );
}

#[test]
fn persona_file_references_resolve_on_disk() {
    // Per supervisor request: catch the case where someone moves or
    // renames a persona file later without updating the orchestrator
    // skill. Each of the 7 personas must exist under
    // references/personas/<name>.md in *both* mirrors.
    const PERSONAS: &[&str] = &[
        "correctness",
        "testing",
        "maintainability",
        "project-standards",
        "security",
        "performance",
        "adversarial",
    ];

    for (label, skill_path) in [
        ("claude", claude_skill_path()),
        ("codex", codex_skill_path()),
    ] {
        let personas_dir = skill_path
            .parent()
            .expect("skill parent")
            .join("references")
            .join("personas");
        for persona in PERSONAS {
            let p = personas_dir.join(format!("{persona}.md"));
            assert!(
                p.exists(),
                "[{label}] persona file missing at {} — either add it \
                 or update SKILL.md to stop referencing it",
                p.display()
            );
        }
    }
}

#[test]
fn base_sha_helper_from_unit_3_is_referenced() {
    // Unit 4 consumes Unit 3's output. Pin that the helper is mentioned
    // so the handoff doesn't silently break if base_sha resolution moves.
    let body = read(&claude_skill_path());
    assert!(
        body.contains("base_sha") || body.contains("base SHA"),
        "SKILL.md must reference base_sha input and its resolution path"
    );
    // And specifically that the Unit 3 helper crate is named, so a
    // reader can find the Rust implementation without guessing.
    assert!(
        body.contains("cas_store::code_review::base_sha")
            || body.contains("code_review::base_sha::resolve")
            || body.contains("crates/cas-store/src/code_review/base_sha.rs"),
        "SKILL.md must point at the Unit 3 base-sha helper so callers \
         can find the Rust implementation"
    );
}
