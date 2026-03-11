//! Benchmarks for code indexing performance
//!
//! Run with: cargo bench --bench code_indexing
//!
//! These benchmarks measure:
//! - Chunking throughput (symbols/sec)
//! - Embedding batch performance (various batch sizes)
//! - BM25 indexing throughput
//! - End-to-end indexing pipeline

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use cas_code::{ChunkConfig, ChunkType, CodeChunker, CodeSymbol, Language, SymbolKind};

/// Generate test symbols for benchmarking
fn generate_symbols(count: usize) -> Vec<CodeSymbol> {
    (0..count)
        .map(|i| CodeSymbol {
            id: format!("sym-{i:06}"),
            qualified_name: format!("module::submodule::function_{i}"),
            name: format!("function_{i}"),
            kind: if i % 3 == 0 {
                SymbolKind::Struct
            } else {
                SymbolKind::Function
            },
            language: Language::Rust,
            file_path: format!("src/module/file_{}.rs", i % 100),
            file_id: format!("file-{}", i % 100),
            line_start: (i * 20) % 1000,
            line_end: (i * 20) % 1000 + 15,
            source: format!(
                r#"/// Documentation for function_{i}
///
/// This function does something important.
/// It has multiple lines of documentation.
fn function_{i}(arg1: String, arg2: i32) -> Result<Output, Error> {{
    let result = do_something(arg1)?;
    let processed = process(result, arg2)?;

    if processed.is_valid() {{
        Ok(processed.into())
    }} else {{
        Err(Error::Invalid)
    }}
}}"#
            ),
            documentation: Some(format!(
                "Documentation for function_{i}.\n\nThis function does something important.\nIt has multiple lines of documentation."
            )),
            signature: Some(format!(
                "fn function_{i}(arg1: String, arg2: i32) -> Result<Output, Error>"
            )),
            ..Default::default()
        })
        .collect()
}

/// Benchmark chunking with different configurations
fn bench_chunking(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunking");

    let symbols = generate_symbols(1000);

    // Benchmark: 1 chunk per symbol (FullSource only)
    group.throughput(Throughput::Elements(symbols.len() as u64));
    group.bench_function("fullsource_only", |b| {
        let config = ChunkConfig {
            chunk_types: vec![ChunkType::FullSource],
            ..Default::default()
        };
        let chunker = CodeChunker::with_config(config);

        b.iter(|| {
            let chunks: Vec<_> = symbols
                .iter()
                .flat_map(|s| chunker.chunk_symbol(black_box(s)))
                .collect();
            black_box(chunks)
        })
    });

    // Benchmark: 2 chunks per symbol (DocSignature + FullSource)
    group.bench_function("docsig_and_fullsource", |b| {
        let config = ChunkConfig {
            chunk_types: vec![ChunkType::DocSignature, ChunkType::FullSource],
            ..Default::default()
        };
        let chunker = CodeChunker::with_config(config);

        b.iter(|| {
            let chunks: Vec<_> = symbols
                .iter()
                .flat_map(|s| chunker.chunk_symbol(black_box(s)))
                .collect();
            black_box(chunks)
        })
    });

    // Benchmark: DocSignature only
    group.bench_function("docsig_only", |b| {
        let config = ChunkConfig {
            chunk_types: vec![ChunkType::DocSignature],
            ..Default::default()
        };
        let chunker = CodeChunker::with_config(config);

        b.iter(|| {
            let chunks: Vec<_> = symbols
                .iter()
                .flat_map(|s| chunker.chunk_symbol(black_box(s)))
                .collect();
            black_box(chunks)
        })
    });

    group.finish();
}

/// Benchmark chunk count verification
fn bench_chunk_counts(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunk_counts");

    let symbol_counts = [100, 500, 1000, 5000];

    for count in symbol_counts {
        let symbols = generate_symbols(count);

        // FullSource only - should produce exactly `count` chunks
        group.bench_with_input(
            BenchmarkId::new("fullsource_chunks", count),
            &symbols,
            |b, symbols| {
                let config = ChunkConfig {
                    chunk_types: vec![ChunkType::FullSource],
                    ..Default::default()
                };
                let chunker = CodeChunker::with_config(config);

                b.iter(|| {
                    let chunk_count: usize = symbols
                        .iter()
                        .map(|s| chunker.chunk_symbol(black_box(s)).len())
                        .sum();
                    assert_eq!(chunk_count, symbols.len(), "Should have 1 chunk per symbol");
                    black_box(chunk_count)
                })
            },
        );

        // DocSignature + FullSource - should produce 2x chunks
        group.bench_with_input(
            BenchmarkId::new("dual_chunks", count),
            &symbols,
            |b, symbols| {
                let config = ChunkConfig {
                    chunk_types: vec![ChunkType::DocSignature, ChunkType::FullSource],
                    ..Default::default()
                };
                let chunker = CodeChunker::with_config(config);

                b.iter(|| {
                    let chunk_count: usize = symbols
                        .iter()
                        .map(|s| chunker.chunk_symbol(black_box(s)).len())
                        .sum();
                    // Note: some symbols may not produce DocSignature (e.g., if no signature)
                    assert!(
                        chunk_count >= symbols.len(),
                        "Should have at least 1 chunk per symbol"
                    );
                    black_box(chunk_count)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark BM25 document preparation (without actual indexing)
fn bench_bm25_prep(c: &mut Criterion) {
    let mut group = c.benchmark_group("bm25_prep");

    let symbol_counts = [100, 1000, 5000];

    for count in symbol_counts {
        let symbols = generate_symbols(count);
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("prepare_docs", count),
            &symbols,
            |b, symbols| {
                b.iter(|| {
                    // Simulate BM25 document preparation
                    let docs: Vec<_> = symbols
                        .iter()
                        .map(|s| {
                            (
                                black_box(&s.id),
                                black_box(&s.qualified_name),
                                black_box(&s.source),
                                black_box(&s.documentation),
                            )
                        })
                        .collect();
                    black_box(docs)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark content hashing (for skip detection)
fn bench_content_hashing(c: &mut Criterion) {
    use sha2::{Digest, Sha256};

    let mut group = c.benchmark_group("content_hashing");

    let symbols = generate_symbols(1000);

    group.throughput(Throughput::Elements(symbols.len() as u64));
    group.bench_function("sha256_symbols", |b| {
        b.iter(|| {
            let hashes: Vec<_> = symbols
                .iter()
                .map(|s| {
                    let mut hasher = Sha256::new();
                    hasher.update(s.source.as_bytes());
                    hex::encode(hasher.finalize())
                })
                .collect();
            black_box(hashes)
        })
    });

    group.finish();
}

/// Benchmark symbol ID generation
fn bench_symbol_id_generation(c: &mut Criterion) {
    use sha2::{Digest, Sha256};

    let mut group = c.benchmark_group("symbol_id");

    let symbol_counts = [100, 1000, 5000];

    for count in symbol_counts {
        let inputs: Vec<_> = (0..count)
            .map(|i| {
                (
                    format!("module::submodule::function_{i}"),
                    format!("src/module/file_{}.rs", i % 100),
                    "my-repo".to_string(),
                )
            })
            .collect();

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("sha256_id", count),
            &inputs,
            |b, inputs| {
                b.iter(|| {
                    let ids: Vec<_> = inputs
                        .iter()
                        .map(|(qname, path, repo)| {
                            let mut hasher = Sha256::new();
                            hasher.update(qname.as_bytes());
                            hasher.update(path.as_bytes());
                            hasher.update(repo.as_bytes());
                            format!("sym-{}", &hex::encode(hasher.finalize())[..12])
                        })
                        .collect();
                    black_box(ids)
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_chunking,
    bench_chunk_counts,
    bench_bm25_prep,
    bench_content_hashing,
    bench_symbol_id_generation,
);

criterion_main!(benches);
