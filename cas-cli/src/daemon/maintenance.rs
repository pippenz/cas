use chrono::Utc;

use crate::daemon::decay::{
    apply_memory_decay, auto_prune, run_consolidation, run_entity_summary_update,
};
use crate::daemon::indexing::generate_bm25_index;
use crate::daemon::observation::process_observations;
use crate::daemon::{DaemonConfig, DaemonRunResult};
use crate::error::CasError;

/// Run a single maintenance cycle.
pub fn run_maintenance(config: &DaemonConfig) -> Result<DaemonRunResult, CasError> {
    use crate::store::{open_agent_store, open_store, open_task_store};
    use crate::types::TaskStatus;

    let started_at = Utc::now();
    let mut errors = Vec::new();
    let mut observations_processed = 0;
    let mut consolidations_applied = 0;
    let mut entries_pruned = 0;
    let mut decay_applied = 0;
    let mut entries_indexed = 0;
    let mut indexing_errors = Vec::new();
    let mut agents_cleaned = 0;
    let mut agents_purged = 0;
    let mut tasks_interrupted = 0;
    let mut worktrees_cleaned = 0;

    let store = open_store(&config.cas_root)?;

    if config.process_observations {
        match process_observations(&store, config) {
            Ok(count) => observations_processed = count,
            Err(error) => errors.push(format!("Observation processing failed: {error}")),
        }
    }

    if config.index_bm25 {
        match generate_bm25_index(&store, config) {
            Ok(result) => {
                entries_indexed = result.indexed;
                for (id, error) in result.errors {
                    indexing_errors.push(format!("{id}: {error}"));
                }
            }
            Err(error) => errors.push(format!("BM25 indexing failed: {error}")),
        }
    }

    if config.apply_decay {
        match apply_memory_decay(&store) {
            Ok(count) => decay_applied = count,
            Err(error) => errors.push(format!("Memory decay failed: {error}")),
        }
    }

    if config.consolidate_memories {
        match run_consolidation(&store, config) {
            Ok(count) => consolidations_applied = count,
            Err(error) => errors.push(format!("Consolidation failed: {error}")),
        }
    }

    if config.auto_prune {
        match auto_prune(&store) {
            Ok(count) => entries_pruned = count,
            Err(error) => errors.push(format!("Auto-prune failed: {error}")),
        }
    }

    let mut entity_summaries_updated = 0;
    if config.update_entity_summaries {
        match run_entity_summary_update(&store, &config.cas_root) {
            Ok(count) => entity_summaries_updated = count,
            Err(error) => errors.push(format!("Entity summary update failed: {error}")),
        }
    }

    if let Ok(agent_store) = open_agent_store(&config.cas_root) {
        if let Ok(stale_agents) = agent_store.list_stale(600) {
            for agent in &stale_agents {
                let held_tasks = agent_store.list_agent_leases(&agent.id).unwrap_or_default();
                let agent_id = agent.id.clone();

                if agent_store.mark_stale(&agent_id).is_ok() {
                    agents_cleaned += 1;

                    if !held_tasks.is_empty() {
                        if let Ok(task_store) = open_task_store(&config.cas_root) {
                            for lease in &held_tasks {
                                if let Ok(mut task) = task_store.get(&lease.task_id) {
                                    if task.status == TaskStatus::InProgress {
                                        let timestamp = Utc::now().format("%Y-%m-%d %H:%M");
                                        let note = format!(
                                            "[{}] ⚠️ INTERRUPTED Agent {} timed out while task was in progress",
                                            timestamp,
                                            &agent_id[..12.min(agent_id.len())]
                                        );
                                        if task.notes.is_empty() {
                                            task.notes = note;
                                        } else {
                                            task.notes = format!("{}\n\n{}", task.notes, note);
                                        }
                                        task.updated_at = Utc::now();
                                        if task_store.update(&task).is_ok() {
                                            tasks_interrupted += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let _ = agent_store.reclaim_expired_leases();

        if config.agent_purge_age_hours > 0 {
            let purge_cutoff =
                Utc::now() - chrono::Duration::hours(config.agent_purge_age_hours as i64);
            if let Ok(all_agents) = agent_store.list(None) {
                for agent in all_agents {
                    if matches!(
                        agent.status,
                        crate::types::AgentStatus::Stale | crate::types::AgentStatus::Shutdown
                    ) && agent.last_heartbeat < purge_cutoff
                        && agent_store.unregister(&agent.id).is_ok()
                    {
                        agents_purged += 1;
                    }
                }
            }
        }
    }

    match cleanup_orphaned_worktrees(config) {
        Ok(count) => worktrees_cleaned = count,
        Err(error) => errors.push(format!("Worktree cleanup failed: {error}")),
    }

    let ended_at = Utc::now();
    let duration_secs = (ended_at - started_at).num_milliseconds() as f64 / 1000.0;

    Ok(DaemonRunResult {
        started_at,
        ended_at,
        duration_secs,
        observations_processed,
        consolidations_applied,
        entries_pruned,
        decay_applied,
        entries_indexed,
        indexing_errors,
        entity_summaries_updated,
        agents_cleaned,
        agents_purged,
        tasks_interrupted,
        worktrees_cleaned,
        errors,
    })
}

/// Clean up orphaned worktrees.
fn cleanup_orphaned_worktrees(config: &DaemonConfig) -> Result<usize, CasError> {
    use crate::config::Config;
    use crate::store::{open_agent_store, open_task_store, open_worktree_store};
    use crate::types::{AgentStatus, TaskStatus};
    use crate::worktree::{WorktreeConfig, WorktreeManager};

    let cas_config = Config::load(&config.cas_root)?;
    let wt_config = cas_config.worktrees();

    if !wt_config.enabled {
        return Ok(0);
    }

    let worktree_store = open_worktree_store(&config.cas_root)?;
    let task_store = open_task_store(&config.cas_root)?;
    let agent_store = open_agent_store(&config.cas_root)?;

    let active_worktrees = worktree_store.list_active()?;
    let mut cleaned = 0;

    for mut worktree in active_worktrees {
        let mut is_orphan = !worktree.path.exists();

        if !is_orphan {
            if let Some(epic_id) = &worktree.epic_id {
                if let Ok(epic) = task_store.get(epic_id) {
                    if matches!(epic.status, TaskStatus::Closed) {
                        is_orphan = true;
                    }
                }
            }
        }

        if !is_orphan {
            if let Some(agent_id) = &worktree.created_by_agent {
                if let Ok(agent) = agent_store.get(agent_id) {
                    if matches!(agent.status, AgentStatus::Stale | AgentStatus::Shutdown) {
                        is_orphan = true;
                    }
                }
            }
        }

        if !is_orphan {
            continue;
        }

        if worktree.path.exists() {
            let manager_config = WorktreeConfig {
                enabled: wt_config.enabled,
                base_path: wt_config.base_path.clone(),
                branch_prefix: wt_config.branch_prefix.clone(),
                auto_merge: wt_config.auto_merge,
                cleanup_on_close: wt_config.cleanup_on_close,
                promote_entries_on_merge: wt_config.promote_entries_on_merge,
            };

            if let Ok(cwd) = std::env::current_dir() {
                if let Ok(manager) = WorktreeManager::new(&cwd, manager_config) {
                    if manager.abandon(&mut worktree, true).is_ok() {
                        worktree.mark_abandoned();
                        worktree.mark_removed();
                        let _ = worktree_store.update(&worktree);
                        cleaned += 1;
                        continue;
                    }
                }
            }
        }

        worktree.mark_abandoned();
        worktree.mark_removed();
        let _ = worktree_store.update(&worktree);
        cleaned += 1;
    }

    Ok(cleaned)
}

/// Run daemon once (for testing or one-shot mode).
pub fn run_once(config: &DaemonConfig) -> Result<DaemonRunResult, CasError> {
    run_maintenance(config)
}
