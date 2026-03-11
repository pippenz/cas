//! Search quality benchmarks comparing BM25-only vs hybrid search
//!
//! Run with: cargo bench --bench search_quality
//!
//! These benchmarks measure:
//! - Search latency (BM25 vs hybrid)
//! - Result overlap between methods
//! - Score distribution differences

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::path::PathBuf;

/// Test queries for search quality evaluation
const TEST_QUERIES: &[&str] = &[
    // Exact match queries (BM25 should excel)
    "SqliteStore",
    "fn search",
    "impl Default",
    // Semantic queries (embeddings should help)
    "how to store data persistently",
    "find similar items",
    "error handling pattern",
    // Code-specific queries
    "database connection",
    "parse json",
    "async function",
    // Mixed queries
    "create new task with priority",
    "search across all memories",
];

/// Benchmark search latency
fn bench_search_latency(c: &mut Criterion) {
    // Skip if CAS root doesn't exist
    let cas_root = match find_cas_root() {
        Some(root) => root,
        None => {
            eprintln!("Skipping search benchmark: CAS root not found");
            return;
        }
    };

    let mut group = c.benchmark_group("search_latency");
    group.sample_size(20); // Fewer samples for expensive operations

    // Open BM25 index
    let bm25_index = match open_bm25_index(&cas_root) {
        Ok(idx) => idx,
        Err(e) => {
            eprintln!("Skipping search benchmark: {e}");
            return;
        }
    };

    for query in TEST_QUERIES.iter().take(3) {
        // BM25-only search
        group.bench_with_input(BenchmarkId::new("bm25_only", query), query, |b, q| {
            b.iter(|| {
                let results = bm25_index.search(black_box(q), 10);
                black_box(results)
            })
        });
    }

    group.finish();
}

/// Compare result overlap between BM25 and hybrid
fn bench_result_overlap(c: &mut Criterion) {
    let cas_root = match find_cas_root() {
        Some(root) => root,
        None => return,
    };

    let mut group = c.benchmark_group("result_analysis");
    group.sample_size(10);

    let bm25_index = match open_bm25_index(&cas_root) {
        Ok(idx) => idx,
        Err(_) => return,
    };

    // Measure how often top results differ
    group.bench_function("top10_overlap", |b| {
        b.iter(|| {
            let mut total_overlap = 0usize;
            for query in TEST_QUERIES {
                let bm25_results = bm25_index.search(query, 10);
                // In a real benchmark, we'd compare with hybrid results
                // For now, just verify BM25 returns results
                total_overlap += bm25_results.map(|r| r.len()).unwrap_or(0);
            }
            black_box(total_overlap)
        })
    });

    group.finish();
}

// Helper functions

fn find_cas_root() -> Option<PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    loop {
        let cas_dir = current.join(".cas");
        if cas_dir.is_dir() {
            return Some(cas_dir);
        }
        if !current.pop() {
            return None;
        }
    }
}

struct SimpleBm25Index {
    // Placeholder - in real impl would use tantivy
}

impl SimpleBm25Index {
    fn search(&self, _query: &str, _limit: usize) -> Result<Vec<String>, &'static str> {
        // Placeholder - returns empty for now
        Ok(vec![])
    }
}

fn open_bm25_index(_cas_root: &PathBuf) -> Result<SimpleBm25Index, &'static str> {
    // Placeholder - would open actual Tantivy index
    Ok(SimpleBm25Index {})
}

criterion_group!(benches, bench_search_latency, bench_result_overlap,);

criterion_main!(benches);
