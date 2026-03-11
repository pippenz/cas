use std::sync::Arc;

use crate::daemon::DaemonConfig;
use crate::error::CasError;
use crate::store::Store;

pub(crate) fn process_observations(
    store: &Arc<dyn Store>,
    config: &DaemonConfig,
) -> Result<usize, CasError> {
    use crate::extraction::{AIExtractor, AIExtractorAsync, AIExtractorConfig};
    use crate::types::{Entry, EntryType};

    let pending = store.list_pending(config.batch_size)?;

    if pending.is_empty() {
        return Ok(0);
    }

    let extractor_config = AIExtractorConfig {
        model: config.model.clone(),
        extract_preferences: false,
        suggest_rules: false,
        ..Default::default()
    };
    let extractor = AIExtractor::new(extractor_config);

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| CasError::Other(format!("Failed to create runtime: {error}")))?;

    let mut extracted_count = 0;

    for entry in &pending {
        if entry.content.len() < 20 {
            store.mark_extracted(&entry.id)?;
            continue;
        }

        match runtime.block_on(extractor.extract_async(entry)) {
            Ok(result) => {
                for learning in result.learnings {
                    if learning.confidence >= 0.6 {
                        let id = store.generate_id()?;
                        let new_entry = Entry {
                            id,
                            entry_type: EntryType::Learning,
                            content: learning.content,
                            tags: learning.tags,
                            importance: learning.confidence,
                            ..Default::default()
                        };
                        let _ = store.add(&new_entry);
                        extracted_count += 1;
                    }
                }
                store.mark_extracted(&entry.id)?;
            }
            Err(error) => {
                eprintln!("cas: Extraction failed for {}: {}", entry.id, error);
                store.mark_extracted(&entry.id)?;
            }
        }
    }

    Ok(extracted_count)
}
