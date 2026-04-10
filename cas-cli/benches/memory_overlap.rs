//! Benchmarks for the pre-insert memory overlap detection workflow.
//!
//! Run with: `cargo bench --bench memory_overlap`
//!
//! Measures the pure Rust scoring hot path (`cas_core::memory::check_overlap`)
//! at several store sizes so the R8 performance budget (<500ms for a single
//! check at the 10k-entry scale, <200ms at the Phase 1 ~50-entry scale) can
//! be validated and regression-tested over time.
//!
//! # Why benchmark the pure scoring path rather than the end-to-end handler
//!
//! `check_overlap` takes pre-fetched facets and does all the token/set/enum
//! work that dominates per-candidate cost. The end-to-end cas_remember path
//! adds BM25 candidate retrieval via Tantivy, which (a) is already exercised
//! by `search_frontmatter_test` and the hybrid-search benches, and (b) is
//! pre-filtered to ≤5 candidates before `check_overlap` ever runs. The hot
//! path that grows with the store size is candidate selection, not scoring;
//! scoring is bounded by the top-N cap.
//!
//! We therefore benchmark two shapes:
//!
//! 1. **Single-check, top-5 candidates** — mirrors real cas_remember calls.
//!    This is the 500ms budget the spec asks for.
//! 2. **Scaling sweep 100/500/1000/5000 candidates** — stresses the scoring
//!    loop with an unrealistic but informative candidate pool to confirm
//!    growth is linear in N (no accidental quadratic behaviour).
//!
//! Results are captured as a task note so future regressions are visible.

use std::collections::HashSet;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand::seq::IndexedRandom;

use cas_core::memory::{CandidateFacets, NewMemoryFacets, check_overlap};

/// Deterministic seed for the synthetic memory corpus. Anchored to the task
/// id so any future regression investigation can reproduce identical data.
const SEED: u64 = 0x_CA54_7210_CA54_7210;

/// Pool of symbol-shaped tokens used to build synthetic facets. Enough
/// variety to avoid trivial collisions while keeping every entry in the
/// same problem space as a realistic memory store.
const SYMBOL_POOL: &[&str] = &[
    "cas-mcp/src/server.rs",
    "cas-mcp/src/tools.rs",
    "cas-core/src/memory/overlap.rs",
    "cas-core/src/dedup.rs",
    "cas-store/src/markdown.rs",
    "cas-cli/src/hybrid_search/mod.rs",
    "ghostty_vt_sys/zig/lib.zig",
    "crates/cas-types/src/entry.rs",
    "handle_remember_request",
    "search_candidates_by_module",
    "posix_lock",
    "reload_policy",
    "tantivy_writer",
    "on_commit_with_delay",
    "frontmatter_parser",
    "boolean_query",
    "2dfe2aa1234567",
    "a77eced1122334",
    "e382deadbeef00",
    "09f15a6cafe000",
];

const TAG_POOL: &[&str] = &[
    "sqlite-wal",
    "ntfs3-fs",
    "mcp-timeout",
    "tantivy-index",
    "hook-context",
    "frontmatter-parse",
    "overlap-detection",
    "boolean-query",
    "search-index",
    "memory-store",
];

const MODULE_POOL: &[&str] = &[
    "cas-mcp",
    "cas-core",
    "cas-cli",
    "cas-store",
    "cas-search",
    "ghostty_vt_sys",
];

const ROOT_CAUSE_POOL: &[&str] = &[
    "race_condition",
    "config_error",
    "platform_incompatibility",
    "missing_validation",
    "off_by_one",
    "stale_cache",
];

const TITLE_WORDS: &[&str] = &[
    "sqlite", "tantivy", "index", "timeout", "deadlock", "parser", "schema", "commit", "reload",
    "worktree", "factory", "supervisor", "worker", "overlap", "detection", "memory", "store",
    "hook", "module", "cache",
];

/// Build one synthetic candidate with a deterministic shape derived from
/// its index. Every Nth candidate is forced to overlap heavily with
/// `reference` so the scoring loop exercises all three decision branches
/// in the same run.
fn build_candidate(
    rng: &mut SmallRng,
    idx: usize,
    reference: &NewMemoryFacets,
) -> CandidateFacets {
    // Shape distribution:
    //   idx % 10 == 0 → clone (high overlap)
    //   idx % 5  == 0 → moderate overlap (title + file_refs clone, rest random)
    //   otherwise     → unrelated random
    if idx.is_multiple_of(10) {
        return CandidateFacets {
            slug: format!("seed-{idx:05}"),
            title: reference.title.clone(),
            description: reference.description.clone(),
            module: reference.module.clone(),
            track: reference.track.clone(),
            root_cause: reference.root_cause.clone(),
            tags: reference.tags.clone(),
            file_refs: reference.file_refs.clone(),
            body_tokens: reference.body_tokens.clone(),
            related_count: 0,
        };
    }

    let title = pick_title(rng);
    let description = pick_description(rng);

    if idx.is_multiple_of(5) {
        // Moderate: share title + one file ref with the reference, diverge
        // on module/track so the 2-3 score band is exercised.
        let mut file_refs: HashSet<String> = reference.file_refs.iter().take(1).cloned().collect();
        file_refs.insert((*SYMBOL_POOL.choose(rng).unwrap()).to_string());
        return CandidateFacets {
            slug: format!("seed-{idx:05}"),
            title: reference.title.clone(),
            description,
            module: Some((*MODULE_POOL.choose(rng).unwrap()).to_string()),
            track: Some("knowledge".to_string()),
            root_cause: Some((*ROOT_CAUSE_POOL.choose(rng).unwrap()).to_string()),
            tags: random_tags(rng, 3),
            file_refs,
            body_tokens: random_tokens(rng, 20),
            related_count: 0,
        };
    }

    CandidateFacets {
        slug: format!("seed-{idx:05}"),
        title,
        description,
        module: Some((*MODULE_POOL.choose(rng).unwrap()).to_string()),
        track: Some(if idx.is_multiple_of(2) { "bug" } else { "knowledge" }.to_string()),
        root_cause: Some((*ROOT_CAUSE_POOL.choose(rng).unwrap()).to_string()),
        tags: random_tags(rng, 3),
        file_refs: random_tokens(rng, 3),
        body_tokens: random_tokens(rng, 20),
        related_count: 0,
    }
}

fn pick_title(rng: &mut SmallRng) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(4);
    for _ in 0..4 {
        parts.push(TITLE_WORDS.choose(rng).unwrap());
    }
    parts.join(" ")
}

fn pick_description(rng: &mut SmallRng) -> String {
    format!(
        "{} regressed after {}",
        pick_title(rng),
        TITLE_WORDS.choose(rng).unwrap()
    )
}

fn random_tags(rng: &mut SmallRng, n: usize) -> HashSet<String> {
    let mut out = HashSet::with_capacity(n);
    for _ in 0..n {
        out.insert((*TAG_POOL.choose(rng).unwrap()).to_string());
    }
    out
}

fn random_tokens(rng: &mut SmallRng, n: usize) -> HashSet<String> {
    let mut out = HashSet::with_capacity(n);
    for _ in 0..n {
        out.insert((*SYMBOL_POOL.choose(rng).unwrap()).to_string());
    }
    out
}

fn reference_memory() -> NewMemoryFacets {
    NewMemoryFacets {
        title: "sqlite wal ntfs3 timeout overlap".to_string(),
        description: "sqlite wal mode hangs on ntfs3 causing mcp timeout".to_string(),
        module: Some("cas-mcp".to_string()),
        track: Some("bug".to_string()),
        root_cause: Some("platform_incompatibility".to_string()),
        tags: ["sqlite-wal", "ntfs3-fs", "mcp-timeout"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        file_refs: [
            "cas-mcp/src/server.rs",
            "cas-cli/src/hybrid_search/mod.rs",
            "cas-store/src/markdown.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        body_tokens: [
            "sqlite",
            "wal",
            "ntfs3",
            "posix_lock",
            "cas-mcp/src/server.rs",
            "cas-cli/src/hybrid_search/mod.rs",
            "cas-store/src/markdown.rs",
            "timeout",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
    }
}

fn build_corpus(size: usize) -> (NewMemoryFacets, Vec<CandidateFacets>) {
    let new = reference_memory();
    let mut rng = SmallRng::seed_from_u64(SEED);
    let candidates: Vec<CandidateFacets> = (0..size)
        .map(|i| build_candidate(&mut rng, i, &new))
        .collect();
    (new, candidates)
}

/// Benchmark the real cas_remember hot path: a single `check_overlap`
/// call against the top-N candidates the BM25 layer actually passes in.
/// This is the number the 500ms budget applies to.
fn bench_check_overlap_top5(c: &mut Criterion) {
    let mut group = c.benchmark_group("check_overlap_top5");
    let (new, full) = build_corpus(1000);
    // Real callers pass at most 5 candidates (salvaged spec); mirror that.
    let top5: Vec<CandidateFacets> = full.into_iter().take(5).collect();

    group.throughput(Throughput::Elements(1));
    group.bench_function("1k_store_top5_candidates", |b| {
        b.iter(|| {
            let decision = check_overlap(black_box(&new), black_box(&top5), false);
            black_box(decision);
        })
    });

    group.finish();
}

/// Benchmark the scoring loop at varying candidate counts. Real callers
/// never pass more than ~5, but sweeping 100..=5000 confirms the loop is
/// linear in the candidate count (no accidental quadratic blow-up).
fn bench_check_overlap_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("check_overlap_scaling");
    for &size in &[100usize, 500, 1000, 5000] {
        let (new, candidates) = build_corpus(size);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &(new, candidates),
            |b, (new_ref, cands)| {
                b.iter(|| {
                    let decision = check_overlap(black_box(new_ref), black_box(cands), false);
                    black_box(decision);
                })
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_check_overlap_top5,
    bench_check_overlap_scaling,
);
criterion_main!(benches);
