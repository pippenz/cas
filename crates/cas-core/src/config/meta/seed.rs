use crate::config::meta::{ConfigMeta, ConfigRegistry, ConfigType, Constraint};

impl ConfigRegistry {
    pub(crate) fn register_defaults(&mut self) {
        // Define section descriptions
        self.section_descriptions.insert("sync", "Rule synchronization to .claude/rules/");
        self.section_descriptions.insert("cloud", "Cloud sync configuration");
        self.section_descriptions.insert("hooks", "Claude Code hook behavior");
        self.section_descriptions.insert("hooks.plan_mode", "Plan mode specific settings");
        self.section_descriptions.insert("tasks", "Task management settings");
        self.section_descriptions.insert("mcp", "MCP server configuration");
        self.section_descriptions.insert("dev", "Development and tracing options");
        self.section_descriptions.insert("embedding", "Background embedding generation");
        self.section_descriptions.insert("notifications", "TUI notification settings");
        self.section_descriptions.insert("notifications.tasks", "Task notification events");
        self.section_descriptions.insert("notifications.entries", "Entry/memory notification events");
        self.section_descriptions.insert("notifications.rules", "Rule notification events");
        self.section_descriptions.insert("notifications.skills", "Skill notification events");

        // ============================================================
        // SYNC SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "sync.enabled",
            section: "sync",
            name: "Enable Sync",
            description: "Whether to automatically sync proven rules to .claude/rules/. When enabled, rules that reach 'Proven' status are copied to the Claude Code rules directory for automatic inclusion.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "sync.target",
            section: "sync",
            name: "Target Directory",
            description: "Target directory for synced rules, relative to project root. Default is .claude/rules/cas which keeps CAS rules separate from manually created rules.",
            value_type: ConfigType::String,
            default: ".claude/rules/cas",
            constraint: Constraint::NotEmpty,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "sync.min_helpful",
            section: "sync",
            name: "Minimum Helpful Votes",
            description: "Minimum number of 'helpful' votes required before a rule is synced. Higher values ensure only well-validated rules are synced.",
            value_type: ConfigType::Int,
            default: "1",
            constraint: Constraint::Min(0),
            advanced: false,
            requires_feature: None,
        });

        // ============================================================
        // CLOUD SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "cloud.auto_sync",
            section: "cloud",
            name: "Auto Sync",
            description: "Automatically sync changes to CAS Cloud when logged in. Sync happens in the background at the configured interval.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "cloud.interval_secs",
            section: "cloud",
            name: "Sync Interval",
            description: "How often to sync with CAS Cloud, in seconds. Lower values provide faster sync but more network usage.",
            value_type: ConfigType::Int,
            default: "300",
            constraint: Constraint::Range(30, 3600),
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "cloud.pull_on_start",
            section: "cloud",
            name: "Pull on Start",
            description: "Pull latest changes from CAS Cloud when the MCP server starts. Ensures you have the latest data from other devices.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "cloud.max_retries",
            section: "cloud",
            name: "Max Retries",
            description: "Maximum number of retry attempts for failed sync operations before giving up.",
            value_type: ConfigType::Int,
            default: "5",
            constraint: Constraint::Range(1, 20),
            advanced: true,
            requires_feature: None,
        });

        // ============================================================
        // HOOKS SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "hooks.capture_enabled",
            section: "hooks",
            name: "Enable Capture",
            description: "Capture observations from Claude Code tool calls. When enabled, Write/Edit/Bash operations are recorded for later processing.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.capture_tools",
            section: "hooks",
            name: "Capture Tools",
            description: "Comma-separated list of tool names to capture observations from. Default captures file operations and shell commands.",
            value_type: ConfigType::StringList,
            default: "Write,Edit,Bash",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.inject_context",
            section: "hooks",
            name: "Inject Context",
            description: "Inject relevant CAS context at session start. Provides Claude with helpful memories, tasks, and rules.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.context_limit",
            section: "hooks",
            name: "Context Entry Limit",
            description: "Maximum number of memory entries to include in context injection. Higher values provide more context but use more tokens.",
            value_type: ConfigType::Int,
            default: "5",
            constraint: Constraint::Range(1, 50),
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.generate_summaries",
            section: "hooks",
            name: "Generate Summaries",
            description: "Generate AI summaries at session end. Requires the 'ai-extraction' feature to be enabled.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: Some("ai-extraction"),
        });

        self.register(ConfigMeta {
            key: "hooks.token_budget",
            section: "hooks",
            name: "Token Budget",
            description: "Maximum tokens to use for context injection. Set to 0 for unlimited. Higher values allow more context but increase prompt size.",
            value_type: ConfigType::Int,
            default: "4000",
            constraint: Constraint::Min(0),
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.ai_context",
            section: "hooks",
            name: "AI Context",
            description: "Use AI to prioritize context items instead of heuristics. Requires claude CLI to be available.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: Some("ai-extraction"),
        });

        self.register(ConfigMeta {
            key: "hooks.ai_model",
            section: "hooks",
            name: "AI Model",
            description: "Model to use for AI context prioritization. Haiku is recommended for speed and cost.",
            value_type: ConfigType::String,
            default: "claude-haiku-4-5",
            constraint: Constraint::NotEmpty,
            advanced: true,
            requires_feature: Some("ai-extraction"),
        });

        self.register(ConfigMeta {
            key: "hooks.minimal_start",
            section: "hooks",
            name: "Minimal Start",
            description: "Start sessions with minimal context (only blocked tasks and pinned memories). Follows context engineering best practice of starting lean.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        // ============================================================
        // HOOKS.PLAN_MODE SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "hooks.plan_mode.enabled",
            section: "hooks.plan_mode",
            name: "Enable Plan Mode",
            description: "Use plan-aware context when Claude is in planning mode. Provides more comprehensive task and memory information.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.plan_mode.token_budget",
            section: "hooks.plan_mode",
            name: "Plan Token Budget",
            description: "Token budget for plan mode context. Typically higher than execution mode to allow for comprehensive planning information.",
            value_type: ConfigType::Int,
            default: "8000",
            constraint: Constraint::Min(0),
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.plan_mode.task_limit",
            section: "hooks.plan_mode",
            name: "Task Limit",
            description: "Maximum number of tasks to show in plan mode context.",
            value_type: ConfigType::Int,
            default: "15",
            constraint: Constraint::Range(1, 100),
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.plan_mode.show_dependencies",
            section: "hooks.plan_mode",
            name: "Show Dependencies",
            description: "Include task dependency trees in plan mode context. Helps understand task relationships.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.plan_mode.include_closed",
            section: "hooks.plan_mode",
            name: "Include Closed Tasks",
            description: "Include recently closed tasks in plan mode for reference. Useful for understanding recent work.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "hooks.plan_mode.semantic_search",
            section: "hooks.plan_mode",
            name: "Semantic Search",
            description: "Search for related memories using semantic similarity in plan mode.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        // ============================================================
        // TASKS SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "tasks.commit_nudge_on_close",
            section: "tasks",
            name: "Commit Nudge",
            description: "Prompt to commit changes when closing a task. Encourages atomic commits tied to task completion.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        // ============================================================
        // MCP SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "mcp.enabled",
            section: "mcp",
            name: "MCP Enabled",
            description: "Whether MCP server mode is enabled. Set automatically during initialization when MCP setup is chosen.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        // ============================================================
        // DEV SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "dev.dev_mode",
            section: "dev",
            name: "Dev Mode",
            description: "Enable development mode with enhanced tracing and diagnostics. Useful for debugging CAS behavior.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "dev.trace_commands",
            section: "dev",
            name: "Trace Commands",
            description: "Trace CLI command executions in dev mode.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "dev.trace_store_ops",
            section: "dev",
            name: "Trace Store Operations",
            description: "Trace store operations (add, update, delete, get) in dev mode.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "dev.trace_claude_api",
            section: "dev",
            name: "Trace Claude API",
            description: "Trace Claude API calls with full prompts and responses in dev mode.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "dev.trace_hooks",
            section: "dev",
            name: "Trace Hooks",
            description: "Trace hook events in dev mode.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "dev.trace_retention_days",
            section: "dev",
            name: "Trace Retention",
            description: "Days to retain traces before automatic cleanup.",
            value_type: ConfigType::Int,
            default: "7",
            constraint: Constraint::Range(1, 365),
            advanced: true,
            requires_feature: None,
        });

        // ============================================================
        // EMBEDDING SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "embedding.enabled",
            section: "embedding",
            name: "Enable Embeddings",
            description: "Enable background embedding generation for semantic search. Embeddings are generated automatically for new entries.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "embedding.batch_size",
            section: "embedding",
            name: "Batch Size",
            description: "Number of entries to embed in a single batch. Larger batches are more GPU-efficient but use more memory.",
            value_type: ConfigType::Int,
            default: "16",
            constraint: Constraint::Range(1, 128),
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "embedding.max_per_run",
            section: "embedding",
            name: "Max Per Run",
            description: "Maximum embeddings to generate per daemon run. Limits work done in each background cycle.",
            value_type: ConfigType::Int,
            default: "100",
            constraint: Constraint::Range(1, 1000),
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "embedding.interval_secs",
            section: "embedding",
            name: "Interval",
            description: "Interval between embedding runs in seconds. Lower values generate embeddings faster but use more CPU.",
            value_type: ConfigType::Int,
            default: "120",
            constraint: Constraint::Range(10, 3600),
            advanced: true,
            requires_feature: None,
        });

        // ============================================================
        // NOTIFICATIONS SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "notifications.enabled",
            section: "notifications",
            name: "Enable Notifications",
            description: "Enable TUI notifications for CAS events like task creation, completion, and memory additions.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.sound_enabled",
            section: "notifications",
            name: "Sound Enabled",
            description: "Play terminal bell sound when notifications appear.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.display_duration_secs",
            section: "notifications",
            name: "Display Duration",
            description: "How long to display notifications in seconds before auto-dismiss.",
            value_type: ConfigType::Int,
            default: "5",
            constraint: Constraint::Range(1, 60),
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.max_visible",
            section: "notifications",
            name: "Max Visible",
            description: "Maximum number of notifications to display at once.",
            value_type: ConfigType::Int,
            default: "3",
            constraint: Constraint::Range(1, 10),
            advanced: false,
            requires_feature: None,
        });

        // ============================================================
        // NOTIFICATIONS.TASKS SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "notifications.tasks.on_created",
            section: "notifications.tasks",
            name: "Task Created",
            description: "Show notification when a task is created.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.tasks.on_started",
            section: "notifications.tasks",
            name: "Task Started",
            description: "Show notification when a task is started.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.tasks.on_closed",
            section: "notifications.tasks",
            name: "Task Closed",
            description: "Show notification when a task is closed/completed.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.tasks.on_updated",
            section: "notifications.tasks",
            name: "Task Updated",
            description: "Show notification when a task is updated. Disabled by default as it can be noisy.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        // ============================================================
        // NOTIFICATIONS.ENTRIES SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "notifications.entries.on_added",
            section: "notifications.entries",
            name: "Entry Added",
            description: "Show notification when a memory entry is added.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.entries.on_updated",
            section: "notifications.entries",
            name: "Entry Updated",
            description: "Show notification when a memory entry is updated. Disabled by default.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.entries.on_deleted",
            section: "notifications.entries",
            name: "Entry Deleted",
            description: "Show notification when a memory entry is deleted.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        // ============================================================
        // NOTIFICATIONS.RULES SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "notifications.rules.on_created",
            section: "notifications.rules",
            name: "Rule Created",
            description: "Show notification when a rule is created.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.rules.on_promoted",
            section: "notifications.rules",
            name: "Rule Promoted",
            description: "Show notification when a rule is promoted to Proven status.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.rules.on_demoted",
            section: "notifications.rules",
            name: "Rule Demoted",
            description: "Show notification when a rule is demoted. Disabled by default.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

        // ============================================================
        // NOTIFICATIONS.SKILLS SECTION
        // ============================================================
        self.register(ConfigMeta {
            key: "notifications.skills.on_created",
            section: "notifications.skills",
            name: "Skill Created",
            description: "Show notification when a skill is created.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.skills.on_enabled",
            section: "notifications.skills",
            name: "Skill Enabled",
            description: "Show notification when a skill is enabled.",
            value_type: ConfigType::Bool,
            default: "true",
            constraint: Constraint::None,
            advanced: false,
            requires_feature: None,
        });

        self.register(ConfigMeta {
            key: "notifications.skills.on_disabled",
            section: "notifications.skills",
            name: "Skill Disabled",
            description: "Show notification when a skill is disabled. Disabled by default.",
            value_type: ConfigType::Bool,
            default: "false",
            constraint: Constraint::None,
            advanced: true,
            requires_feature: None,
        });

    }
}
