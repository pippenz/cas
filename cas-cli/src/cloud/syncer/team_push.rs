use std::time::Instant;

use crate::cloud::syncer::{CloudSyncer, GroupedQueuedItems, SyncResult, TeamPushResponse};
use crate::cloud::{EntityType, QueuedSync, SyncOperation, get_project_canonical_id};
use crate::error::CasError;
use chrono::Utc;

impl CloudSyncer {
    pub fn push_team(&self, team_id: &str) -> Result<SyncResult, CasError> {
        let mut result = SyncResult::default();
        let start = Instant::now();

        if !self.is_available() {
            return Ok(result);
        }

        // Drain queue items with matching team_id
        let queued = self.queue.drain_by_team(team_id, self.config.max_retries)?;

        if queued.is_empty() {
            result.duration_ms = start.elapsed().as_millis() as u64;
            return Ok(result);
        }

        let token = self
            .cloud_config
            .token
            .as_ref()
            .ok_or_else(|| CasError::Other("Not logged in".to_string()))?;

        // Group by entity type and operation
        let grouped = self.group_queued_items(&queued);

        // Build payload for upserts
        let mut payload = serde_json::Map::new();
        if !grouped.upsert_entries.is_empty() {
            payload.insert(
                "entries".to_string(),
                serde_json::json!(grouped.upsert_entries),
            );
        }
        if !grouped.upsert_tasks.is_empty() {
            payload.insert("tasks".to_string(), serde_json::json!(grouped.upsert_tasks));
        }
        if !grouped.upsert_rules.is_empty() {
            payload.insert("rules".to_string(), serde_json::json!(grouped.upsert_rules));
        }
        if !grouped.upsert_skills.is_empty() {
            payload.insert(
                "skills".to_string(),
                serde_json::json!(grouped.upsert_skills),
            );
        }
        if !grouped.upsert_sessions.is_empty() {
            payload.insert(
                "sessions".to_string(),
                serde_json::json!(grouped.upsert_sessions),
            );
        }
        if !grouped.upsert_verifications.is_empty() {
            payload.insert(
                "verifications".to_string(),
                serde_json::json!(grouped.upsert_verifications),
            );
        }
        if !grouped.upsert_events.is_empty() {
            payload.insert(
                "events".to_string(),
                serde_json::json!(grouped.upsert_events),
            );
        }
        if !grouped.upsert_prompts.is_empty() {
            payload.insert(
                "prompts".to_string(),
                serde_json::json!(grouped.upsert_prompts),
            );
        }
        if !grouped.upsert_file_changes.is_empty() {
            payload.insert(
                "file_changes".to_string(),
                serde_json::json!(grouped.upsert_file_changes),
            );
        }
        if !grouped.upsert_commit_links.is_empty() {
            payload.insert(
                "commit_links".to_string(),
                serde_json::json!(grouped.upsert_commit_links),
            );
        }
        if !grouped.upsert_agents.is_empty() {
            payload.insert(
                "agents".to_string(),
                serde_json::json!(grouped.upsert_agents),
            );
        }
        if !grouped.upsert_worktrees.is_empty() {
            payload.insert(
                "worktrees".to_string(),
                serde_json::json!(grouped.upsert_worktrees),
            );
        }

        // Include project_canonical_id (required for project scoping)
        let project_id = get_project_canonical_id()
            .ok_or_else(|| CasError::Other("Cannot sync: not inside a CAS project directory".to_string()))?;
        payload.insert(
            "project_canonical_id".to_string(),
            serde_json::json!(project_id),
        );

        // Track if we have upserts to push
        let has_upserts = !grouped.upsert_entries.is_empty()
            || !grouped.upsert_tasks.is_empty()
            || !grouped.upsert_rules.is_empty()
            || !grouped.upsert_skills.is_empty()
            || !grouped.upsert_sessions.is_empty()
            || !grouped.upsert_verifications.is_empty()
            || !grouped.upsert_events.is_empty()
            || !grouped.upsert_prompts.is_empty()
            || !grouped.upsert_file_changes.is_empty()
            || !grouped.upsert_commit_links.is_empty()
            || !grouped.upsert_agents.is_empty()
            || !grouped.upsert_worktrees.is_empty();

        let has_deletes = !grouped.delete_entries.is_empty()
            || !grouped.delete_tasks.is_empty()
            || !grouped.delete_rules.is_empty()
            || !grouped.delete_skills.is_empty()
            || !grouped.delete_sessions.is_empty()
            || !grouped.delete_verifications.is_empty()
            || !grouped.delete_events.is_empty()
            || !grouped.delete_prompts.is_empty()
            || !grouped.delete_file_changes.is_empty()
            || !grouped.delete_commit_links.is_empty()
            || !grouped.delete_agents.is_empty()
            || !grouped.delete_worktrees.is_empty();

        let mut last_error = None;

        // POST upserts to team endpoint with retry (only if there are upserts)
        if has_upserts {
            let push_url = format!(
                "{}/api/teams/{}/sync/push",
                self.cloud_config.endpoint, team_id
            );

            for attempt in 0..3 {
                if attempt > 0 {
                    std::thread::sleep(self.config.backoff_duration(attempt as u32));
                }

                let json_bytes = serde_json::to_vec(&payload)
                    .map_err(|e| CasError::Other(format!("JSON serialization failed: {e}")))?;
                let compressed = Self::gzip_json(&json_bytes)?;

                let response = ureq::post(&push_url)
                    .timeout(self.config.timeout)
                    .set("Authorization", &format!("Bearer {token}"))
                    .set("Content-Type", "application/json")
                    .set("Content-Encoding", "gzip")
                    .send_bytes(&compressed);

                match response {
                    Ok(resp) => {
                        if resp.status() == 200 || resp.status() == 201 {
                            // Parse response for sync counts
                            if let Ok(body) = resp.into_json::<TeamPushResponse>() {
                                result.pushed_entries = body.synced.entries;
                                result.pushed_tasks = body.synced.tasks;
                                result.pushed_rules = body.synced.rules;
                                result.pushed_skills = body.synced.skills;
                                result.pushed_sessions = body.synced.sessions;
                                result.pushed_verifications = body.synced.verifications;
                                result.pushed_events = body.synced.events;
                                result.pushed_prompts = body.synced.prompts;
                                result.pushed_file_changes = body.synced.file_changes;
                                result.pushed_commit_links = body.synced.commit_links;
                                result.pushed_agents = body.synced.agents;
                                result.pushed_worktrees = body.synced.worktrees;
                            } else {
                                // If parsing fails, count what we sent
                                result.pushed_entries = grouped.upsert_entries.len();
                                result.pushed_tasks = grouped.upsert_tasks.len();
                                result.pushed_rules = grouped.upsert_rules.len();
                                result.pushed_skills = grouped.upsert_skills.len();
                                result.pushed_sessions = grouped.upsert_sessions.len();
                                result.pushed_verifications = grouped.upsert_verifications.len();
                                result.pushed_events = grouped.upsert_events.len();
                                result.pushed_prompts = grouped.upsert_prompts.len();
                                result.pushed_file_changes = grouped.upsert_file_changes.len();
                                result.pushed_commit_links = grouped.upsert_commit_links.len();
                                result.pushed_agents = grouped.upsert_agents.len();
                                result.pushed_worktrees = grouped.upsert_worktrees.len();
                            }
                            last_error = None;
                            break;
                        } else {
                            let status = resp.status();
                            let body = resp.into_string().unwrap_or_default();
                            last_error = Some(CasError::Other(format!(
                                "Team push failed with status {status}: {body}"
                            )));
                            // Don't retry 4xx errors
                            if (400..500).contains(&status) {
                                break;
                            }
                        }
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        let body = resp.into_string().unwrap_or_default();
                        last_error = Some(CasError::Other(format!(
                            "Team push failed with status {code}: {body}"
                        )));
                        if (400..500).contains(&code) {
                            break;
                        }
                    }
                    Err(ureq::Error::Transport(e)) => {
                        last_error = Some(CasError::Other(format!("Network error: {e}")));
                    }
                }
            }
        }

        if let Some(err) = last_error {
            // Re-enqueue items on failure
            for item in &queued {
                let _ = self.queue.enqueue_for_team(
                    item.entity_type,
                    &item.entity_id,
                    item.operation,
                    item.payload.as_deref(),
                    team_id,
                );
            }
            result.errors.push(err.to_string());
        } else if has_deletes {
            // Process deletes (after successful upserts or if no upserts)
            let (deleted_count, delete_errors) = self.send_team_deletes(team_id, &grouped, token);
            // Track successful deletes (deleted_count is total across all types)
            if deleted_count > 0 {
                // Note: delete counts aren't tracked separately in SyncResult,
                // they're part of the overall push operation
                let _ = deleted_count; // Acknowledge the count
            }
            if !delete_errors.is_empty() {
                result.errors.extend(delete_errors);
            }
        }

        // Update team sync timestamp on success
        if result.errors.is_empty() {
            let _ = self.queue.set_metadata(
                &format!("last_team_push_at_{team_id}"),
                &Utc::now().to_rfc3339(),
            );
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }

    /// Group queued items by entity type and operation
    fn group_queued_items(&self, items: &[QueuedSync]) -> GroupedQueuedItems {
        let mut result = GroupedQueuedItems::default();

        for item in items {
            match item.operation {
                SyncOperation::Upsert => {
                    if let Some(payload) = &item.payload {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
                            match item.entity_type {
                                EntityType::Entry => result.upsert_entries.push(value),
                                EntityType::Task => result.upsert_tasks.push(value),
                                EntityType::Rule => result.upsert_rules.push(value),
                                EntityType::Skill => result.upsert_skills.push(value),
                                EntityType::Session => result.upsert_sessions.push(value),
                                EntityType::Verification => result.upsert_verifications.push(value),
                                EntityType::Event => result.upsert_events.push(value),
                                EntityType::Prompt => result.upsert_prompts.push(value),
                                EntityType::FileChange => result.upsert_file_changes.push(value),
                                EntityType::CommitLink => result.upsert_commit_links.push(value),
                                EntityType::Agent => result.upsert_agents.push(value),
                                EntityType::Worktree => result.upsert_worktrees.push(value),
                            }
                        }
                    }
                }
                SyncOperation::Delete => match item.entity_type {
                    EntityType::Entry => result.delete_entries.push(item.entity_id.clone()),
                    EntityType::Task => result.delete_tasks.push(item.entity_id.clone()),
                    EntityType::Rule => result.delete_rules.push(item.entity_id.clone()),
                    EntityType::Skill => result.delete_skills.push(item.entity_id.clone()),
                    EntityType::Session => result.delete_sessions.push(item.entity_id.clone()),
                    EntityType::Verification => {
                        result.delete_verifications.push(item.entity_id.clone())
                    }
                    EntityType::Event => result.delete_events.push(item.entity_id.clone()),
                    EntityType::Prompt => result.delete_prompts.push(item.entity_id.clone()),
                    EntityType::FileChange => {
                        result.delete_file_changes.push(item.entity_id.clone())
                    }
                    EntityType::CommitLink => {
                        result.delete_commit_links.push(item.entity_id.clone())
                    }
                    EntityType::Agent => result.delete_agents.push(item.entity_id.clone()),
                    EntityType::Worktree => result.delete_worktrees.push(item.entity_id.clone()),
                },
            }
        }

        result
    }

    /// Send team delete requests for each entity type
    fn send_team_deletes(
        &self,
        team_id: &str,
        grouped: &GroupedQueuedItems,
        token: &str,
    ) -> (usize, Vec<String>) {
        let mut deleted = 0;
        let mut errors = Vec::new();

        // Delete entries
        for cas_id in &grouped.delete_entries {
            match self.send_team_delete(team_id, "entries", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("Entry delete {cas_id}: {e}")),
            }
        }

        // Delete tasks
        for cas_id in &grouped.delete_tasks {
            match self.send_team_delete(team_id, "tasks", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("Task delete {cas_id}: {e}")),
            }
        }

        // Delete rules
        for cas_id in &grouped.delete_rules {
            match self.send_team_delete(team_id, "rules", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("Rule delete {cas_id}: {e}")),
            }
        }

        // Delete skills
        for cas_id in &grouped.delete_skills {
            match self.send_team_delete(team_id, "skills", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("Skill delete {cas_id}: {e}")),
            }
        }

        // Delete sessions
        for cas_id in &grouped.delete_sessions {
            match self.send_team_delete(team_id, "sessions", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("Session delete {cas_id}: {e}")),
            }
        }

        // Delete verifications
        for cas_id in &grouped.delete_verifications {
            match self.send_team_delete(team_id, "verifications", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("Verification delete {cas_id}: {e}")),
            }
        }

        // Delete events
        for cas_id in &grouped.delete_events {
            match self.send_team_delete(team_id, "events", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("Event delete {cas_id}: {e}")),
            }
        }

        // Delete prompts
        for cas_id in &grouped.delete_prompts {
            match self.send_team_delete(team_id, "prompts", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("Prompt delete {cas_id}: {e}")),
            }
        }

        // Delete file changes
        for cas_id in &grouped.delete_file_changes {
            match self.send_team_delete(team_id, "file_changes", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("FileChange delete {cas_id}: {e}")),
            }
        }

        // Delete commit links
        for cas_id in &grouped.delete_commit_links {
            match self.send_team_delete(team_id, "commit_links", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("CommitLink delete {cas_id}: {e}")),
            }
        }

        // Delete agents
        for cas_id in &grouped.delete_agents {
            match self.send_team_delete(team_id, "agents", cas_id, token) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("Agent delete {cas_id}: {e}")),
            }
        }

        (deleted, errors)
    }

    /// Send a single team delete request
    fn send_team_delete(
        &self,
        team_id: &str,
        entity_type: &str,
        cas_id: &str,
        token: &str,
    ) -> Result<(), CasError> {
        let delete_url = format!(
            "{}/api/teams/{}/sync/{}/{}",
            self.cloud_config.endpoint, team_id, entity_type, cas_id
        );

        let response = ureq::delete(&delete_url)
            .timeout(self.config.timeout)
            .set("Authorization", &format!("Bearer {token}"))
            .call();

        match response {
            Ok(resp) if resp.status() == 200 || resp.status() == 404 => Ok(()),
            Ok(resp) => {
                let status = resp.status();
                let body = resp.into_string().unwrap_or_default();
                Err(CasError::Other(format!(
                    "Delete failed with status {status}: {body}"
                )))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                Err(CasError::Other(format!(
                    "Delete failed with status {code}: {body}"
                )))
            }
            Err(ureq::Error::Transport(e)) => Err(CasError::Other(format!("Network error: {e}"))),
        }
    }
}
