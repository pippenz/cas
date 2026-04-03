use std::time::Instant;

use crate::cloud::syncer::{
    CloudSyncer, ConflictAction, ConflictResolution, PullResponse, SyncResult, TeamPullResponse,
    UpsertResult,
};
use crate::cloud::get_project_canonical_id;
use crate::error::CasError;
use crate::store::{RuleStore, SkillStore, Store, TaskStore};
use crate::types::{Entry, Rule, Session, Skill, Task};

/// Check whether a raw JSON entity belongs to the current project.
///
/// Returns `true` if the entity should be accepted, `false` if it should be skipped.
///
/// An entity is accepted when:
/// - It has no `project_canonical_id` or `project_id` field (legacy / server not yet scoping)
/// - Its project field is `null`
/// - Its project field matches `current_project_id`
fn entity_matches_project(
    raw: &serde_json::Value,
    current_project_id: &str,
    entity_kind: &str,
) -> bool {
    // Check both field names the server might use
    let project_field = raw
        .get("project_canonical_id")
        .or_else(|| raw.get("project_id"));

    match project_field {
        None => true, // No field present — legacy entity, accept it
        Some(serde_json::Value::Null) => true, // Explicitly null — not scoped, accept it
        Some(serde_json::Value::String(s)) => {
            if s == current_project_id {
                true
            } else {
                let entity_id = raw
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>");
                eprintln!(
                    "[CAS sync] WARNING: skipping {entity_kind} '{entity_id}' from foreign \
                     project '{s}' (expected '{current_project_id}')"
                );
                false
            }
        }
        Some(_) => true, // Unexpected type — don't block on it, accept
    }
}

impl CloudSyncer {
    pub fn pull(
        &self,
        store: &dyn Store,
        task_store: &dyn TaskStore,
        rule_store: &dyn RuleStore,
        skill_store: &dyn SkillStore,
    ) -> Result<SyncResult, CasError> {
        let mut result = SyncResult::default();
        let start = Instant::now();

        if !self.is_available() {
            return Ok(result);
        }

        let token = self
            .cloud_config
            .token
            .as_ref()
            .ok_or_else(|| CasError::Other("Not logged in".to_string()))?;

        // Get last pull timestamp
        let since = self.queue.get_metadata("last_pull_at")?;

        let mut pull_url = format!("{}/api/sync/pull", self.cloud_config.endpoint);
        let mut params = Vec::new();
        if let Some(since) = &since {
            params.push(format!("since={since}"));
        }
        if let Some(project_id) = get_project_canonical_id() {
            params.push(format!("project_id={}", project_id.replace('/', "%2F")));
        }
        if !params.is_empty() {
            pull_url = format!("{pull_url}?{}", params.join("&"));
        }

        let response = ureq::get(&pull_url)
            .timeout(self.config.timeout)
            .set("Authorization", &format!("Bearer {token}"))
            .call();

        let body: PullResponse = match response {
            Ok(resp) => resp
                .into_json()
                .map_err(|e| CasError::Other(format!("Failed to parse response: {e}")))?,
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                return Err(CasError::Other(format!(
                    "Pull failed with status {code}: {body}"
                )));
            }
            Err(ureq::Error::Transport(e)) => {
                return Err(CasError::Other(format!("Network error: {e}")));
            }
        };

        // Get the current project ID once — used for client-side entity validation
        let current_project_id = get_project_canonical_id()
            .unwrap_or_else(|| "unknown".to_string());

        // Process entries
        for raw_entry in body.entries.unwrap_or_default() {
            if !entity_matches_project(&raw_entry, &current_project_id, "entry") {
                continue;
            }
            let remote_entry: Entry = match serde_json::from_value(raw_entry) {
                Ok(e) => e,
                Err(e) => {
                    result.errors.push(format!("Entry deserialize error: {e}"));
                    continue;
                }
            };
            match self.upsert_entry(store, remote_entry) {
                Ok(UpsertResult::Created) | Ok(UpsertResult::Updated) => {
                    result.pulled_entries += 1;
                }
                Ok(UpsertResult::Skipped) => {
                    result.conflicts_resolved += 1;
                }
                Err(e) => {
                    result.errors.push(format!("Entry error: {e}"));
                }
            }
        }

        // Process tasks
        for raw_task in body.tasks.unwrap_or_default() {
            if !entity_matches_project(&raw_task, &current_project_id, "task") {
                continue;
            }
            let remote_task: Task = match serde_json::from_value(raw_task) {
                Ok(t) => t,
                Err(e) => {
                    result.errors.push(format!("Task deserialize error: {e}"));
                    continue;
                }
            };
            match self.upsert_task(task_store, remote_task) {
                Ok(UpsertResult::Created) | Ok(UpsertResult::Updated) => {
                    result.pulled_tasks += 1;
                }
                Ok(UpsertResult::Skipped) => {
                    result.conflicts_resolved += 1;
                }
                Err(e) => {
                    result.errors.push(format!("Task error: {e}"));
                }
            }
        }

        // Process rules
        for raw_rule in body.rules.unwrap_or_default() {
            if !entity_matches_project(&raw_rule, &current_project_id, "rule") {
                continue;
            }
            let remote_rule: Rule = match serde_json::from_value(raw_rule) {
                Ok(r) => r,
                Err(e) => {
                    result.errors.push(format!("Rule deserialize error: {e}"));
                    continue;
                }
            };
            match self.upsert_rule(rule_store, remote_rule) {
                Ok(UpsertResult::Created) | Ok(UpsertResult::Updated) => {
                    result.pulled_rules += 1;
                }
                Ok(UpsertResult::Skipped) => {
                    result.conflicts_resolved += 1;
                }
                Err(e) => {
                    result.errors.push(format!("Rule error: {e}"));
                }
            }
        }

        // Process skills
        for raw_skill in body.skills.unwrap_or_default() {
            if !entity_matches_project(&raw_skill, &current_project_id, "skill") {
                continue;
            }
            let remote_skill: Skill = match serde_json::from_value(raw_skill) {
                Ok(s) => s,
                Err(e) => {
                    result.errors.push(format!("Skill deserialize error: {e}"));
                    continue;
                }
            };
            match self.upsert_skill(skill_store, remote_skill) {
                Ok(UpsertResult::Created) | Ok(UpsertResult::Updated) => {
                    result.pulled_skills += 1;
                }
                Ok(UpsertResult::Skipped) => {
                    result.conflicts_resolved += 1;
                }
                Err(e) => {
                    result.errors.push(format!("Skill error: {e}"));
                }
            }
        }

        // Update last pull timestamp
        if let Some(pulled_at) = body.pulled_at {
            let _ = self.queue.set_metadata("last_pull_at", &pulled_at);
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }

    fn upsert_entry(&self, store: &dyn Store, entry: Entry) -> Result<UpsertResult, CasError> {
        match store.get(&entry.id) {
            Ok(local) => {
                // Compare timestamps for conflict resolution (last-write-wins)
                let local_time = local.last_accessed.unwrap_or(local.created);
                let remote_time = entry.last_accessed.unwrap_or(entry.created);

                if remote_time > local_time {
                    store.update(&entry)?;
                    Ok(UpsertResult::Updated)
                } else {
                    Ok(UpsertResult::Skipped)
                }
            }
            Err(cas_store::StoreError::EntryNotFound(_)) => {
                store.add(&entry)?;
                Ok(UpsertResult::Created)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn upsert_task(&self, store: &dyn TaskStore, task: Task) -> Result<UpsertResult, CasError> {
        match store.get(&task.id) {
            Ok(local) => {
                if task.updated_at > local.updated_at {
                    store.update(&task)?;
                    Ok(UpsertResult::Updated)
                } else {
                    Ok(UpsertResult::Skipped)
                }
            }
            Err(cas_store::StoreError::TaskNotFound(_)) => {
                store.add(&task)?;
                Ok(UpsertResult::Created)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn upsert_rule(&self, store: &dyn RuleStore, rule: Rule) -> Result<UpsertResult, CasError> {
        match store.get(&rule.id) {
            Ok(local) => {
                // Compare by last_accessed or created
                let local_time = local.last_accessed.unwrap_or(local.created);
                let remote_time = rule.last_accessed.unwrap_or(rule.created);

                if remote_time > local_time {
                    store.update(&rule)?;
                    Ok(UpsertResult::Updated)
                } else {
                    Ok(UpsertResult::Skipped)
                }
            }
            Err(cas_store::StoreError::RuleNotFound(_)) => {
                store.add(&rule)?;
                Ok(UpsertResult::Created)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn upsert_skill(&self, store: &dyn SkillStore, skill: Skill) -> Result<UpsertResult, CasError> {
        match store.get(&skill.id) {
            Ok(local) => {
                if skill.updated_at > local.updated_at {
                    store.update(&skill)?;
                    Ok(UpsertResult::Updated)
                } else {
                    Ok(UpsertResult::Skipped)
                }
            }
            Err(cas_store::StoreError::SkillNotFound(_)) => {
                store.add(&skill)?;
                Ok(UpsertResult::Created)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Upsert entry with configurable conflict resolution for team sync
    fn upsert_entry_with_strategy(
        &self,
        store: &dyn Store,
        entry: Entry,
        strategy: ConflictResolution,
    ) -> Result<UpsertResult, CasError> {
        match store.get(&entry.id) {
            Ok(local) => {
                let local_time = local.last_accessed.unwrap_or(local.created);
                let remote_time = entry.last_accessed.unwrap_or(entry.created);

                let action =
                    self.resolve_conflict("entry", &entry.id, local_time, remote_time, strategy);

                match action {
                    ConflictAction::UseRemote => {
                        store.update(&entry)?;
                        Ok(UpsertResult::Updated)
                    }
                    ConflictAction::UseLocal | ConflictAction::Skip => Ok(UpsertResult::Skipped),
                }
            }
            Err(cas_store::StoreError::EntryNotFound(_)) => {
                store.add(&entry)?;
                Ok(UpsertResult::Created)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Upsert task with configurable conflict resolution for team sync
    fn upsert_task_with_strategy(
        &self,
        store: &dyn TaskStore,
        task: Task,
        strategy: ConflictResolution,
    ) -> Result<UpsertResult, CasError> {
        match store.get(&task.id) {
            Ok(local) => {
                let action = self.resolve_conflict(
                    "task",
                    &task.id,
                    local.updated_at,
                    task.updated_at,
                    strategy,
                );

                match action {
                    ConflictAction::UseRemote => {
                        store.update(&task)?;
                        Ok(UpsertResult::Updated)
                    }
                    ConflictAction::UseLocal | ConflictAction::Skip => Ok(UpsertResult::Skipped),
                }
            }
            Err(cas_store::StoreError::TaskNotFound(_)) => {
                store.add(&task)?;
                Ok(UpsertResult::Created)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Upsert rule with configurable conflict resolution for team sync
    fn upsert_rule_with_strategy(
        &self,
        store: &dyn RuleStore,
        rule: Rule,
        strategy: ConflictResolution,
    ) -> Result<UpsertResult, CasError> {
        match store.get(&rule.id) {
            Ok(local) => {
                let local_time = local.last_accessed.unwrap_or(local.created);
                let remote_time = rule.last_accessed.unwrap_or(rule.created);

                let action =
                    self.resolve_conflict("rule", &rule.id, local_time, remote_time, strategy);

                match action {
                    ConflictAction::UseRemote => {
                        store.update(&rule)?;
                        Ok(UpsertResult::Updated)
                    }
                    ConflictAction::UseLocal | ConflictAction::Skip => Ok(UpsertResult::Skipped),
                }
            }
            Err(cas_store::StoreError::RuleNotFound(_)) => {
                store.add(&rule)?;
                Ok(UpsertResult::Created)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Upsert skill with configurable conflict resolution for team sync
    fn upsert_skill_with_strategy(
        &self,
        store: &dyn SkillStore,
        skill: Skill,
        strategy: ConflictResolution,
    ) -> Result<UpsertResult, CasError> {
        match store.get(&skill.id) {
            Ok(local) => {
                let action = self.resolve_conflict(
                    "skill",
                    &skill.id,
                    local.updated_at,
                    skill.updated_at,
                    strategy,
                );

                match action {
                    ConflictAction::UseRemote => {
                        store.update(&skill)?;
                        Ok(UpsertResult::Updated)
                    }
                    ConflictAction::UseLocal | ConflictAction::Skip => Ok(UpsertResult::Skipped),
                }
            }
            Err(cas_store::StoreError::SkillNotFound(_)) => {
                store.add(&skill)?;
                Ok(UpsertResult::Created)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Full sync: push then pull
    pub fn sync(
        &self,
        store: &dyn Store,
        task_store: &dyn TaskStore,
        rule_store: &dyn RuleStore,
        skill_store: &dyn SkillStore,
    ) -> Result<SyncResult, CasError> {
        self.sync_with_sessions(store, task_store, rule_store, skill_store, &[])
    }

    /// Full sync with sessions: push (including sessions) then pull
    pub fn sync_with_sessions(
        &self,
        store: &dyn Store,
        task_store: &dyn TaskStore,
        rule_store: &dyn RuleStore,
        skill_store: &dyn SkillStore,
        sessions: &[Session],
    ) -> Result<SyncResult, CasError> {
        let start = Instant::now();

        // Push first (with sessions)
        let push_result = self.push_with_sessions(sessions)?;

        // Then pull
        let pull_result = self.pull(store, task_store, rule_store, skill_store)?;

        // Combine results
        Ok(SyncResult {
            pushed_entries: push_result.pushed_entries,
            pushed_tasks: push_result.pushed_tasks,
            pushed_rules: push_result.pushed_rules,
            pushed_skills: push_result.pushed_skills,
            pushed_sessions: push_result.pushed_sessions,
            pushed_verifications: push_result.pushed_verifications,
            pushed_events: push_result.pushed_events,
            pushed_prompts: push_result.pushed_prompts,
            pushed_file_changes: push_result.pushed_file_changes,
            pushed_commit_links: push_result.pushed_commit_links,
            pushed_agents: push_result.pushed_agents,
            pushed_worktrees: push_result.pushed_worktrees,
            pulled_entries: pull_result.pulled_entries,
            pulled_tasks: pull_result.pulled_tasks,
            pulled_rules: pull_result.pulled_rules,
            pulled_skills: pull_result.pulled_skills,
            conflicts_resolved: pull_result.conflicts_resolved,
            errors: [push_result.errors, pull_result.errors].concat(),
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Pull team data from cloud and merge into local store
    pub fn pull_team(
        &self,
        team_id: &str,
        store: &dyn Store,
        task_store: &dyn TaskStore,
        rule_store: &dyn RuleStore,
        skill_store: &dyn SkillStore,
    ) -> Result<SyncResult, CasError> {
        let mut result = SyncResult::default();
        let start = Instant::now();

        if !self.is_available() {
            return Ok(result);
        }

        let token = self
            .cloud_config
            .token
            .as_ref()
            .ok_or_else(|| CasError::Other("Not logged in".to_string()))?;

        // Get last pull timestamp for this team
        let since_key = format!("last_team_pull_at_{team_id}");
        let since = self.queue.get_metadata(&since_key)?;

        let mut pull_url = format!(
            "{}/api/teams/{}/sync/pull",
            self.cloud_config.endpoint, team_id
        );
        let mut params = Vec::new();
        if let Some(since) = &since {
            params.push(format!("since={since}"));
        }
        if let Some(project_id) = get_project_canonical_id() {
            params.push(format!("project_id={}", project_id.replace('/', "%2F")));
        }
        if !params.is_empty() {
            pull_url = format!("{pull_url}?{}", params.join("&"));
        }

        let response = ureq::get(&pull_url)
            .timeout(self.config.timeout)
            .set("Authorization", &format!("Bearer {token}"))
            .call();

        let body: TeamPullResponse = match response {
            Ok(resp) => resp
                .into_json()
                .map_err(|e| CasError::Other(format!("Failed to parse team pull response: {e}")))?,
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                return Err(CasError::Other(format!(
                    "Team pull failed with status {code}: {body}"
                )));
            }
            Err(ureq::Error::Transport(e)) => {
                return Err(CasError::Other(format!("Network error: {e}")));
            }
        };

        // Use configured conflict resolution strategy for team sync
        let strategy = self.config.team_conflict_resolution;
        #[cfg(debug_assertions)]
        eprintln!("[CAS sync] Starting team pull: team={team_id} strategy={strategy:?}");

        // Get the current project ID for client-side validation
        let current_project_id = get_project_canonical_id()
            .unwrap_or_else(|| "unknown".to_string());

        // Process entries
        for raw_entry in body.entries.unwrap_or_default() {
            if !entity_matches_project(&raw_entry, &current_project_id, "entry") {
                continue;
            }
            let remote_entry: Entry = match serde_json::from_value(raw_entry) {
                Ok(e) => e,
                Err(e) => {
                    result.errors.push(format!("Entry deserialize error: {e}"));
                    continue;
                }
            };
            match self.upsert_entry_with_strategy(store, remote_entry, strategy) {
                Ok(UpsertResult::Created) | Ok(UpsertResult::Updated) => {
                    result.pulled_entries += 1;
                }
                Ok(UpsertResult::Skipped) => {
                    result.conflicts_resolved += 1;
                }
                Err(e) => {
                    result.errors.push(format!("Entry error: {e}"));
                }
            }
        }

        // Process tasks
        for raw_task in body.tasks.unwrap_or_default() {
            if !entity_matches_project(&raw_task, &current_project_id, "task") {
                continue;
            }
            let remote_task: Task = match serde_json::from_value(raw_task) {
                Ok(t) => t,
                Err(e) => {
                    result.errors.push(format!("Task deserialize error: {e}"));
                    continue;
                }
            };
            match self.upsert_task_with_strategy(task_store, remote_task, strategy) {
                Ok(UpsertResult::Created) | Ok(UpsertResult::Updated) => {
                    result.pulled_tasks += 1;
                }
                Ok(UpsertResult::Skipped) => {
                    result.conflicts_resolved += 1;
                }
                Err(e) => {
                    result.errors.push(format!("Task error: {e}"));
                }
            }
        }

        // Process rules
        for raw_rule in body.rules.unwrap_or_default() {
            if !entity_matches_project(&raw_rule, &current_project_id, "rule") {
                continue;
            }
            let remote_rule: Rule = match serde_json::from_value(raw_rule) {
                Ok(r) => r,
                Err(e) => {
                    result.errors.push(format!("Rule deserialize error: {e}"));
                    continue;
                }
            };
            match self.upsert_rule_with_strategy(rule_store, remote_rule, strategy) {
                Ok(UpsertResult::Created) | Ok(UpsertResult::Updated) => {
                    result.pulled_rules += 1;
                }
                Ok(UpsertResult::Skipped) => {
                    result.conflicts_resolved += 1;
                }
                Err(e) => {
                    result.errors.push(format!("Rule error: {e}"));
                }
            }
        }

        // Process skills
        for raw_skill in body.skills.unwrap_or_default() {
            if !entity_matches_project(&raw_skill, &current_project_id, "skill") {
                continue;
            }
            let remote_skill: Skill = match serde_json::from_value(raw_skill) {
                Ok(s) => s,
                Err(e) => {
                    result.errors.push(format!("Skill deserialize error: {e}"));
                    continue;
                }
            };
            match self.upsert_skill_with_strategy(skill_store, remote_skill, strategy) {
                Ok(UpsertResult::Created) | Ok(UpsertResult::Updated) => {
                    result.pulled_skills += 1;
                }
                Ok(UpsertResult::Skipped) => {
                    result.conflicts_resolved += 1;
                }
                Err(e) => {
                    result.errors.push(format!("Skill error: {e}"));
                }
            }
        }

        // Update team pull timestamp
        if let Some(pulled_at) = body.pulled_at {
            let _ = self.queue.set_metadata(&since_key, &pulled_at);
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::entity_matches_project;
    use serde_json::json;

    #[test]
    fn test_entity_matches_project_no_field() {
        // No project field — legacy entity, always accepted
        let entity = json!({ "id": "e-001", "content": "hello" });
        assert!(entity_matches_project(&entity, "github.com/owner/repo", "entry"));
    }

    #[test]
    fn test_entity_matches_project_null_field() {
        // Null project_canonical_id — not scoped to any project, accepted
        let entity = json!({ "id": "e-001", "project_canonical_id": null });
        assert!(entity_matches_project(&entity, "github.com/owner/repo", "entry"));
    }

    #[test]
    fn test_entity_matches_project_matching() {
        // Matching project — accepted
        let entity = json!({ "id": "e-001", "project_canonical_id": "github.com/owner/repo" });
        assert!(entity_matches_project(&entity, "github.com/owner/repo", "entry"));
    }

    #[test]
    fn test_entity_matches_project_foreign() {
        // Different project — rejected (returns false)
        let entity = json!({ "id": "e-001", "project_canonical_id": "github.com/other/repo" });
        assert!(!entity_matches_project(&entity, "github.com/owner/repo", "entry"));
    }

    #[test]
    fn test_entity_matches_project_id_field_alias() {
        // Also checks `project_id` field as an alias
        let entity = json!({ "id": "t-abc", "project_id": "github.com/owner/repo" });
        assert!(entity_matches_project(&entity, "github.com/owner/repo", "task"));
    }

    #[test]
    fn test_entity_matches_project_id_field_foreign() {
        let entity = json!({ "id": "t-abc", "project_id": "github.com/other/repo" });
        assert!(!entity_matches_project(&entity, "github.com/owner/repo", "task"));
    }

    #[test]
    fn test_entity_matches_project_null_project_id() {
        // Null project_id — accepted (not scoped)
        let entity = json!({ "id": "t-abc", "project_id": null });
        assert!(entity_matches_project(&entity, "github.com/owner/repo", "task"));
    }

    #[test]
    fn test_entity_matches_local_project() {
        // local: prefix IDs work the same way
        let entity = json!({ "id": "p-001", "project_canonical_id": "local:abcd1234ef567890" });
        assert!(entity_matches_project(&entity, "local:abcd1234ef567890", "entry"));
        assert!(!entity_matches_project(&entity, "local:0000000000000000", "entry"));
    }
}
