use crate::config::{
    meta, CloudSyncConfig, Config, HookConfig,
};
use crate::error::CoreError;

impl Config {
    /// Set a config value by key
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), CoreError> {
        // Validate using metadata if available
        if let Some(meta) = meta::registry().get(key) {
            meta.validate(value).map_err(CoreError::Parse)?;
        }

        match key {
            // Sync section
            "sync.enabled" => {
                self.sync.enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "sync.target" => {
                self.sync.target = value.to_string();
            }
            "sync.min_helpful" => {
                self.sync.min_helpful = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            // Cloud section
            "cloud.auto_sync" => {
                let cloud = self.cloud.get_or_insert_with(CloudSyncConfig::default);
                cloud.auto_sync = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "cloud.interval_secs" => {
                let cloud = self.cloud.get_or_insert_with(CloudSyncConfig::default);
                cloud.interval_secs = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "cloud.pull_on_start" => {
                let cloud = self.cloud.get_or_insert_with(CloudSyncConfig::default);
                cloud.pull_on_start = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "cloud.max_retries" => {
                let cloud = self.cloud.get_or_insert_with(CloudSyncConfig::default);
                cloud.max_retries = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            // Hooks section
            "hooks.capture_enabled" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.capture_enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "hooks.capture_tools" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.capture_tools = value.split(',').map(|s| s.trim().to_string()).collect();
            }
            "hooks.inject_context" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.inject_context = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "hooks.context_limit" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.context_limit = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "hooks.generate_summaries" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.generate_summaries = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "hooks.token_budget" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.token_budget = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "hooks.ai_context" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.ai_context = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "hooks.ai_model" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.ai_model = value.to_string();
            }
            "hooks.minimal_start" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.minimal_start = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            // Hooks.plan_mode section
            "hooks.plan_mode.enabled" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.plan_mode.enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "hooks.plan_mode.token_budget" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.plan_mode.token_budget = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "hooks.plan_mode.task_limit" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.plan_mode.task_limit = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "hooks.plan_mode.show_dependencies" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.plan_mode.show_dependencies = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "hooks.plan_mode.include_closed" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.plan_mode.include_closed = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "hooks.plan_mode.semantic_search" => {
                let hooks = self.hooks.get_or_insert_with(HookConfig::default);
                hooks.plan_mode.semantic_search = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            // Tasks section
            "tasks.commit_nudge_on_close" => {
                let tasks = self.tasks.get_or_insert_with(TasksConfig::default);
                tasks.commit_nudge_on_close = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "tasks.block_exit_on_open" => {
                let tasks = self.tasks.get_or_insert_with(TasksConfig::default);
                tasks.block_exit_on_open = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            // MCP section
            "mcp.enabled" => {
                let mcp = self.mcp.get_or_insert_with(McpConfig::default);
                mcp.enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            // Dev section
            "dev.dev_mode" => {
                let dev = self.dev.get_or_insert_with(DevConfig::default);
                dev.dev_mode = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "dev.trace_commands" => {
                let dev = self.dev.get_or_insert_with(DevConfig::default);
                dev.trace_commands = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "dev.trace_store_ops" => {
                let dev = self.dev.get_or_insert_with(DevConfig::default);
                dev.trace_store_ops = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "dev.trace_claude_api" => {
                let dev = self.dev.get_or_insert_with(DevConfig::default);
                dev.trace_claude_api = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "dev.trace_hooks" => {
                let dev = self.dev.get_or_insert_with(DevConfig::default);
                dev.trace_hooks = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "dev.trace_retention_days" => {
                let dev = self.dev.get_or_insert_with(DevConfig::default);
                dev.trace_retention_days = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            // Embedding section
            "embedding.enabled" => {
                let embedding = self.embedding.get_or_insert_with(EmbeddingConfig::default);
                embedding.enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "embedding.model" => {
                let embedding = self.embedding.get_or_insert_with(EmbeddingConfig::default);
                embedding.model = value.to_string();
            }
            "embedding.reranker" => {
                let embedding = self.embedding.get_or_insert_with(EmbeddingConfig::default);
                // Empty string or "none" disables the reranker
                if value.is_empty() || value.to_lowercase() == "none" {
                    embedding.reranker = None;
                } else {
                    embedding.reranker = Some(value.to_string());
                }
            }
            "embedding.batch_size" => {
                let embedding = self.embedding.get_or_insert_with(EmbeddingConfig::default);
                embedding.batch_size = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "embedding.max_per_run" => {
                let embedding = self.embedding.get_or_insert_with(EmbeddingConfig::default);
                embedding.max_per_run = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "embedding.interval_secs" => {
                let embedding = self.embedding.get_or_insert_with(EmbeddingConfig::default);
                embedding.interval_secs = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            // Notifications section
            "notifications.enabled" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.sound_enabled" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.sound_enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.display_duration_secs" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.display_duration_secs = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "notifications.max_visible" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.max_visible = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            // Notifications.tasks section
            "notifications.tasks.on_created" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.tasks.on_created = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.tasks.on_started" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.tasks.on_started = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.tasks.on_closed" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.tasks.on_closed = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.tasks.on_updated" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.tasks.on_updated = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            // Notifications.entries section
            "notifications.entries.on_added" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.entries.on_added = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.entries.on_updated" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.entries.on_updated = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.entries.on_deleted" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.entries.on_deleted = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            // Notifications.rules section
            "notifications.rules.on_created" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.rules.on_created = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.rules.on_promoted" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.rules.on_promoted = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.rules.on_demoted" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.rules.on_demoted = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            // Notifications.skills section
            "notifications.skills.on_created" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.skills.on_created = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.skills.on_enabled" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.skills.on_enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "notifications.skills.on_disabled" => {
                let notifications = self.notifications.get_or_insert_with(NotificationConfig::default);
                notifications.skills.on_disabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            // Agent section
            "agent.name" => {
                let agent = self.agent.get_or_insert_with(AgentConfig::default);
                agent.name = value.to_string();
            }
            "agent.max_concurrent_tasks" => {
                let agent = self.agent.get_or_insert_with(AgentConfig::default);
                agent.max_concurrent_tasks = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "agent.capabilities" => {
                let agent = self.agent.get_or_insert_with(AgentConfig::default);
                agent.capabilities = value.split(',').map(|s| s.trim().to_string()).collect();
            }
            // Coordination section
            "coordination.mode" => {
                let coordination = self.coordination.get_or_insert_with(CoordinationConfig::default);
                coordination.mode = match value.to_lowercase().as_str() {
                    "local" => CoordinationMode::Local,
                    "cloud" => CoordinationMode::Cloud,
                    _ => return Err(CoreError::Parse(format!("Invalid coordination mode: {} (expected 'local' or 'cloud')", value))),
                };
            }
            "coordination.cloud_url" => {
                let coordination = self.coordination.get_or_insert_with(CoordinationConfig::default);
                coordination.cloud_url = Some(value.to_string());
            }
            // Lease section
            "lease.default_duration_mins" => {
                let lease = self.lease.get_or_insert_with(LeaseConfig::default);
                lease.default_duration_mins = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "lease.max_duration_mins" => {
                let lease = self.lease.get_or_insert_with(LeaseConfig::default);
                lease.max_duration_mins = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "lease.heartbeat_interval_secs" => {
                let lease = self.lease.get_or_insert_with(LeaseConfig::default);
                lease.heartbeat_interval_secs = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "lease.expiry_grace_secs" => {
                let lease = self.lease.get_or_insert_with(LeaseConfig::default);
                lease.expiry_grace_secs = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            // Verification section
            "verification.enabled" => {
                let verification = self.verification.get_or_insert_with(VerificationConfig::default);
                verification.enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "verification.model" => {
                let verification = self.verification.get_or_insert_with(VerificationConfig::default);
                verification.model = value.to_string();
            }
            "verification.force_bypass_allowed" => {
                let verification = self.verification.get_or_insert_with(VerificationConfig::default);
                verification.force_bypass_allowed = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "verification.timeout_secs" => {
                let verification = self.verification.get_or_insert_with(VerificationConfig::default);
                verification.timeout_secs = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            "verification.patterns.todo_comments" => {
                let verification = self.verification.get_or_insert_with(VerificationConfig::default);
                verification.patterns.todo_comments = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "verification.patterns.temporal_shortcuts" => {
                let verification = self.verification.get_or_insert_with(VerificationConfig::default);
                verification.patterns.temporal_shortcuts = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "verification.patterns.stub_implementations" => {
                let verification = self.verification.get_or_insert_with(VerificationConfig::default);
                verification.patterns.stub_implementations = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "verification.patterns.empty_error_handlers" => {
                let verification = self.verification.get_or_insert_with(VerificationConfig::default);
                verification.patterns.empty_error_handlers = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "verification.patterns.custom" => {
                let verification = self.verification.get_or_insert_with(VerificationConfig::default);
                verification.patterns.custom = value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
            // Worktrees section
            "worktrees.enabled" => {
                let worktrees = self.worktrees.get_or_insert_with(WorktreesConfig::default);
                worktrees.enabled = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "worktrees.base_path" => {
                let worktrees = self.worktrees.get_or_insert_with(WorktreesConfig::default);
                worktrees.base_path = value.to_string();
            }
            "worktrees.branch_prefix" => {
                let worktrees = self.worktrees.get_or_insert_with(WorktreesConfig::default);
                worktrees.branch_prefix = value.to_string();
            }
            "worktrees.auto_merge" => {
                let worktrees = self.worktrees.get_or_insert_with(WorktreesConfig::default);
                worktrees.auto_merge = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "worktrees.cleanup_on_close" => {
                let worktrees = self.worktrees.get_or_insert_with(WorktreesConfig::default);
                worktrees.cleanup_on_close = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "worktrees.promote_entries_on_merge" => {
                let worktrees = self.worktrees.get_or_insert_with(WorktreesConfig::default);
                worktrees.promote_entries_on_merge = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            // Factory section
            "factory.warn_stale_assignment" => {
                let factory = self.factory.get_or_insert_with(FactoryConfig::default);
                factory.warn_stale_assignment = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "factory.block_stale_assignment" => {
                let factory = self.factory.get_or_insert_with(FactoryConfig::default);
                factory.block_stale_assignment = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid boolean value: {}", value))
                })?;
            }
            "factory.stale_threshold_commits" => {
                let factory = self.factory.get_or_insert_with(FactoryConfig::default);
                factory.stale_threshold_commits = value.parse().map_err(|_| {
                    CoreError::Parse(format!("Invalid integer value: {}", value))
                })?;
            }
            _ => {
                return Err(CoreError::Other(format!("Unknown config key: {}", key)));
            }
        }
        Ok(())
    }

    /// List all config keys and values
    pub fn list(&self) -> Vec<(String, String)> {
        let cloud = self.cloud.clone().unwrap_or_default();
        let hooks = self.hooks.clone().unwrap_or_default();
        let tasks = self.tasks.clone().unwrap_or_default();
        let mcp = self.mcp.clone().unwrap_or_default();
        let dev = self.dev.clone().unwrap_or_default();
        let embedding = self.embedding.clone().unwrap_or_default();
        let notifications = self.notifications.clone().unwrap_or_default();
        vec![
            // Sync section
            ("sync.enabled".to_string(), self.sync.enabled.to_string()),
            ("sync.target".to_string(), self.sync.target.clone()),
            ("sync.min_helpful".to_string(), self.sync.min_helpful.to_string()),
            // Cloud section
            ("cloud.auto_sync".to_string(), cloud.auto_sync.to_string()),
            ("cloud.interval_secs".to_string(), cloud.interval_secs.to_string()),
            ("cloud.pull_on_start".to_string(), cloud.pull_on_start.to_string()),
            ("cloud.max_retries".to_string(), cloud.max_retries.to_string()),
            // Hooks section
            ("hooks.capture_enabled".to_string(), hooks.capture_enabled.to_string()),
            ("hooks.capture_tools".to_string(), hooks.capture_tools.join(",")),
            ("hooks.inject_context".to_string(), hooks.inject_context.to_string()),
            ("hooks.context_limit".to_string(), hooks.context_limit.to_string()),
            ("hooks.generate_summaries".to_string(), hooks.generate_summaries.to_string()),
            ("hooks.token_budget".to_string(), hooks.token_budget.to_string()),
            ("hooks.ai_context".to_string(), hooks.ai_context.to_string()),
            ("hooks.ai_model".to_string(), hooks.ai_model.clone()),
            ("hooks.minimal_start".to_string(), hooks.minimal_start.to_string()),
            // Hooks.plan_mode section
            ("hooks.plan_mode.enabled".to_string(), hooks.plan_mode.enabled.to_string()),
            ("hooks.plan_mode.token_budget".to_string(), hooks.plan_mode.token_budget.to_string()),
            ("hooks.plan_mode.task_limit".to_string(), hooks.plan_mode.task_limit.to_string()),
            ("hooks.plan_mode.show_dependencies".to_string(), hooks.plan_mode.show_dependencies.to_string()),
            ("hooks.plan_mode.include_closed".to_string(), hooks.plan_mode.include_closed.to_string()),
            ("hooks.plan_mode.semantic_search".to_string(), hooks.plan_mode.semantic_search.to_string()),
            // Tasks section
            ("tasks.commit_nudge_on_close".to_string(), tasks.commit_nudge_on_close.to_string()),
            ("tasks.block_exit_on_open".to_string(), tasks.block_exit_on_open.to_string()),
            // MCP section
            ("mcp.enabled".to_string(), mcp.enabled.to_string()),
            // Dev section
            ("dev.dev_mode".to_string(), dev.dev_mode.to_string()),
            ("dev.trace_commands".to_string(), dev.trace_commands.to_string()),
            ("dev.trace_store_ops".to_string(), dev.trace_store_ops.to_string()),
            ("dev.trace_claude_api".to_string(), dev.trace_claude_api.to_string()),
            ("dev.trace_hooks".to_string(), dev.trace_hooks.to_string()),
            ("dev.trace_retention_days".to_string(), dev.trace_retention_days.to_string()),
            // Embedding section
            ("embedding.enabled".to_string(), embedding.enabled.to_string()),
            ("embedding.model".to_string(), embedding.model.clone()),
            ("embedding.reranker".to_string(), embedding.reranker.clone().unwrap_or_default()),
            ("embedding.batch_size".to_string(), embedding.batch_size.to_string()),
            ("embedding.max_per_run".to_string(), embedding.max_per_run.to_string()),
            ("embedding.interval_secs".to_string(), embedding.interval_secs.to_string()),
            // Notifications section
            ("notifications.enabled".to_string(), notifications.enabled.to_string()),
            ("notifications.sound_enabled".to_string(), notifications.sound_enabled.to_string()),
            ("notifications.display_duration_secs".to_string(), notifications.display_duration_secs.to_string()),
            ("notifications.max_visible".to_string(), notifications.max_visible.to_string()),
            // Notifications.tasks section
            ("notifications.tasks.on_created".to_string(), notifications.tasks.on_created.to_string()),
            ("notifications.tasks.on_started".to_string(), notifications.tasks.on_started.to_string()),
            ("notifications.tasks.on_closed".to_string(), notifications.tasks.on_closed.to_string()),
            ("notifications.tasks.on_updated".to_string(), notifications.tasks.on_updated.to_string()),
            // Notifications.entries section
            ("notifications.entries.on_added".to_string(), notifications.entries.on_added.to_string()),
            ("notifications.entries.on_updated".to_string(), notifications.entries.on_updated.to_string()),
            ("notifications.entries.on_deleted".to_string(), notifications.entries.on_deleted.to_string()),
            // Notifications.rules section
            ("notifications.rules.on_created".to_string(), notifications.rules.on_created.to_string()),
            ("notifications.rules.on_promoted".to_string(), notifications.rules.on_promoted.to_string()),
            ("notifications.rules.on_demoted".to_string(), notifications.rules.on_demoted.to_string()),
            // Notifications.skills section
            ("notifications.skills.on_created".to_string(), notifications.skills.on_created.to_string()),
            ("notifications.skills.on_enabled".to_string(), notifications.skills.on_enabled.to_string()),
            ("notifications.skills.on_disabled".to_string(), notifications.skills.on_disabled.to_string()),
            // Agent section
            ("agent.name".to_string(), self.agent.clone().unwrap_or_default().name),
            ("agent.max_concurrent_tasks".to_string(), self.agent.clone().unwrap_or_default().max_concurrent_tasks.to_string()),
            ("agent.capabilities".to_string(), self.agent.clone().unwrap_or_default().capabilities.join(",")),
            // Coordination section
            ("coordination.mode".to_string(), match self.coordination.clone().unwrap_or_default().mode {
                CoordinationMode::Local => "local".to_string(),
                CoordinationMode::Cloud => "cloud".to_string(),
            }),
            ("coordination.cloud_url".to_string(), self.coordination.clone().unwrap_or_default().cloud_url.unwrap_or_default()),
            // Lease section
            ("lease.default_duration_mins".to_string(), self.lease.clone().unwrap_or_default().default_duration_mins.to_string()),
            ("lease.max_duration_mins".to_string(), self.lease.clone().unwrap_or_default().max_duration_mins.to_string()),
            ("lease.heartbeat_interval_secs".to_string(), self.lease.clone().unwrap_or_default().heartbeat_interval_secs.to_string()),
            ("lease.expiry_grace_secs".to_string(), self.lease.clone().unwrap_or_default().expiry_grace_secs.to_string()),
            // Verification section
            ("verification.enabled".to_string(), self.verification.clone().unwrap_or_default().enabled.to_string()),
            ("verification.model".to_string(), self.verification.clone().unwrap_or_default().model),
            ("verification.force_bypass_allowed".to_string(), self.verification.clone().unwrap_or_default().force_bypass_allowed.to_string()),
            ("verification.timeout_secs".to_string(), self.verification.clone().unwrap_or_default().timeout_secs.to_string()),
            ("verification.patterns.todo_comments".to_string(), self.verification.clone().unwrap_or_default().patterns.todo_comments.to_string()),
            ("verification.patterns.temporal_shortcuts".to_string(), self.verification.clone().unwrap_or_default().patterns.temporal_shortcuts.to_string()),
            ("verification.patterns.stub_implementations".to_string(), self.verification.clone().unwrap_or_default().patterns.stub_implementations.to_string()),
            ("verification.patterns.empty_error_handlers".to_string(), self.verification.clone().unwrap_or_default().patterns.empty_error_handlers.to_string()),
            ("verification.patterns.custom".to_string(), self.verification.clone().unwrap_or_default().patterns.custom.join(",")),
            // Worktrees section
            ("worktrees.enabled".to_string(), self.worktrees.clone().unwrap_or_default().enabled.to_string()),
            ("worktrees.base_path".to_string(), self.worktrees.clone().unwrap_or_default().base_path),
            ("worktrees.branch_prefix".to_string(), self.worktrees.clone().unwrap_or_default().branch_prefix),
            ("worktrees.auto_merge".to_string(), self.worktrees.clone().unwrap_or_default().auto_merge.to_string()),
            ("worktrees.cleanup_on_close".to_string(), self.worktrees.clone().unwrap_or_default().cleanup_on_close.to_string()),
            ("worktrees.promote_entries_on_merge".to_string(), self.worktrees.clone().unwrap_or_default().promote_entries_on_merge.to_string()),
            // Factory section
            ("factory.warn_stale_assignment".to_string(), self.factory.clone().unwrap_or_default().warn_stale_assignment.to_string()),
            ("factory.block_stale_assignment".to_string(), self.factory.clone().unwrap_or_default().block_stale_assignment.to_string()),
            ("factory.stale_threshold_commits".to_string(), self.factory.clone().unwrap_or_default().stale_threshold_commits.to_string()),
        ]
    }

    /// Get notification config with defaults
    pub fn notifications(&self) -> NotificationConfig {
        self.notifications.clone().unwrap_or_default()
    }

    /// Check if notifications are enabled
    pub fn notifications_enabled(&self) -> bool {
        self.notifications.as_ref().map(|n| n.enabled).unwrap_or(true)
    }

    /// Get agent config with defaults
    pub fn agent(&self) -> AgentConfig {
        self.agent.clone().unwrap_or_default()
    }

    /// Get coordination config with defaults
    pub fn coordination(&self) -> CoordinationConfig {
        self.coordination.clone().unwrap_or_default()
    }

    /// Get lease config with defaults
    pub fn lease(&self) -> LeaseConfig {
        self.lease.clone().unwrap_or_default()
    }

    /// Get verification config with defaults
    pub fn verification(&self) -> VerificationConfig {
        self.verification.clone().unwrap_or_default()
    }

    /// Get tasks config with defaults
    pub fn tasks(&self) -> TasksConfig {
        self.tasks.clone().unwrap_or_default()
    }

    /// Get worktrees config with defaults
    pub fn worktrees(&self) -> WorktreesConfig {
        self.worktrees.clone().unwrap_or_default()
    }

    /// Check if worktrees are enabled
    pub fn worktrees_enabled(&self) -> bool {
        self.worktrees.as_ref().map(|w| w.enabled).unwrap_or(false)
    }

    /// Get factory config with defaults
    pub fn factory(&self) -> FactoryConfig {
        self.factory.clone().unwrap_or_default()
    }

    /// Check if exit blocking is enabled
    pub fn block_exit_on_open(&self) -> bool {
        self.tasks.as_ref().map(|t| t.block_exit_on_open).unwrap_or(true)
    }

    /// Check if verification is enabled
    pub fn verification_enabled(&self) -> bool {
        self.verification.as_ref().map(|v| v.enabled).unwrap_or(true)
    }
}
}
