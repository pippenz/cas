use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::daemon::{CodeIndexResult, CodeWatcher, DaemonConfig, EmbeddingResult, WatchEvent};
use crate::error::CasError;
use crate::store::Store;

/// Run embedding-only maintenance cycle (no-op - daemon removed).
pub fn run_embedding_cycle(_config: &DaemonConfig) -> Result<EmbeddingResult, CasError> {
    Ok(EmbeddingResult::default())
}

pub(crate) fn generate_bm25_index(
    store: &Arc<dyn Store>,
    config: &DaemonConfig,
) -> Result<crate::hybrid_search::IndexingResult, CasError> {
    use crate::hybrid_search::{BackgroundIndexer, IndexingConfig};

    let indexer = match BackgroundIndexer::open(&config.cas_root) {
        Ok(indexer) => indexer,
        Err(error) => {
            return Ok(crate::hybrid_search::IndexingResult {
                indexed: 0,
                errors: vec![("index".to_string(), error.to_string())],
            });
        }
    };

    let index_config = IndexingConfig {
        batch_size: config.index_batch_size,
        max_per_run: config.index_max_per_run,
    };

    indexer.process_pending(store.as_ref(), &index_config)
}

/// Run indexing-only maintenance cycle (for incremental BM25 updates).
pub fn run_indexing_cycle(
    config: &DaemonConfig,
) -> Result<crate::hybrid_search::IndexingResult, CasError> {
    use crate::store::open_store;

    if !config.index_bm25 {
        return Ok(crate::hybrid_search::IndexingResult::default());
    }

    let store = open_store(&config.cas_root)?;
    generate_bm25_index(&store, config)
}

/// Index changed code files (called by file watcher or periodic task).
pub fn index_code_files(files: &[PathBuf], cas_root: &Path) -> Result<CodeIndexResult, CasError> {
    use cas_code::Language;
    use cas_code::parser::MultiLanguageParser;
    use sha2::{Digest, Sha256};

    use crate::store::open_code_store;

    if files.is_empty() {
        return Ok(CodeIndexResult::default());
    }

    let code_store = open_code_store(cas_root)?;

    let mut result = CodeIndexResult::default();
    let mut parser = match MultiLanguageParser::new() {
        Ok(parser) => parser,
        Err(error) => {
            result
                .errors
                .push(format!("Failed to create parser: {error}"));
            return Ok(result);
        }
    };

    for file_path in files {
        let extension = file_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_lowercase())
            .unwrap_or_default();
        let language = Language::from_extension(&extension);

        if !parser.supports(language) {
            continue;
        }

        let content = match std::fs::read_to_string(file_path) {
            Ok(content) => content,
            Err(error) => {
                result
                    .errors
                    .push(format!("{}: {}", file_path.display(), error));
                continue;
            }
        };

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let content_hash = hex::encode(hasher.finalize());

        let repo_name = file_path
            .parent()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        if let Ok(Some(existing)) =
            code_store.get_file_by_path(&repo_name, &file_path.to_string_lossy())
        {
            if existing.content_hash == content_hash {
                continue;
            }
        }

        match parser.parse_file(file_path, &content, &repo_name) {
            Ok(parse_result) => {
                let now = chrono::Utc::now();
                let file_path_str = file_path.to_string_lossy().to_string();
                let file_id = code_store.generate_file_id_for(&repo_name, &file_path_str);

                let _ = code_store.delete_symbols_in_file(&file_id);

                let file = cas_code::CodeFile {
                    id: file_id.clone(),
                    path: file_path_str,
                    repository: repo_name.clone(),
                    language,
                    size: content.len(),
                    line_count: content.lines().count(),
                    commit_hash: None,
                    content_hash,
                    created: now,
                    updated: now,
                    scope: "project".to_string(),
                };

                if let Err(error) = code_store.add_file(&file) {
                    result
                        .errors
                        .push(format!("{}: {}", file_path.display(), error));
                    continue;
                }

                let symbol_count = parse_result.symbols.len();
                let symbols: Vec<cas_code::CodeSymbol> = parse_result
                    .symbols
                    .into_iter()
                    .map(|mut symbol| {
                        symbol.file_id = file_id.clone();
                        symbol.id = code_store.generate_symbol_id_for(
                            &symbol.qualified_name,
                            &symbol.file_path,
                            &symbol.repository,
                        );
                        symbol
                    })
                    .collect();

                if let Err(_batch_err) = code_store.add_symbols_batch(&symbols) {
                    // Fall back to individual inserts on batch failure
                    for symbol in &symbols {
                        if let Err(error) = code_store.add_symbol(symbol) {
                            result
                                .errors
                                .push(format!("Symbol {}: {}", symbol.name, error));
                        }
                    }
                }

                result.symbols_indexed += symbol_count;
                result.files_indexed += 1;
            }
            Err(error) => {
                result
                    .errors
                    .push(format!("{}: {}", file_path.display(), error));
            }
        }
    }

    Ok(result)
}

/// Run code indexing cycle (for periodic background indexing).
pub fn run_code_index_cycle(
    watcher: &CodeWatcher,
    cas_root: &Path,
) -> Result<CodeIndexResult, CasError> {
    use crate::store::open_code_store;

    let mut result = CodeIndexResult::default();
    let mut deleted_paths: Vec<PathBuf> = Vec::new();

    while let Some(event) = watcher.try_recv() {
        match event {
            WatchEvent::Modified(_path) => {}
            WatchEvent::Deleted(path) => deleted_paths.push(path),
            WatchEvent::Error(message) => {
                eprintln!("[CAS] Watcher error: {message}");
                result.errors.push(format!("Watcher: {message}"));
            }
        }
    }

    if !deleted_paths.is_empty() {
        if let Ok(code_store) = open_code_store(cas_root) {
            for path in &deleted_paths {
                let repo_name = path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let path_str = path.to_string_lossy();
                if let Ok(Some(file)) = code_store.get_file_by_path(&repo_name, &path_str) {
                    if code_store.delete_file(&file.id).is_ok() {
                        result.files_deleted += 1;
                    }
                }
            }
        }
    }

    let pending_files = watcher.take_pending();
    if !pending_files.is_empty() {
        let index_result = index_code_files(&pending_files, cas_root)?;
        result.files_indexed = index_result.files_indexed;
        result.symbols_indexed = index_result.symbols_indexed;
        result.errors.extend(index_result.errors);
    }

    Ok(result)
}
