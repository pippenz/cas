use cas_factory::AutoPromptConfig;
use serde::{Deserialize, Serialize};

/// Sync configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Whether auto-sync is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Target directory for synced rules (relative to project root)
    #[serde(default = "default_target")]
    pub target: String,

    /// Minimum helpful votes before syncing
    #[serde(default = "default_min_helpful")]
    pub min_helpful: i32,
}

/// Task configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksConfig {
    /// Nudge to commit changes when closing a task
    #[serde(default)]
    pub commit_nudge_on_close: bool,

    /// Block agent exit while open tasks remain (claimed tasks, epic subtasks, session-created)
    #[serde(default = "default_true")]
    pub block_exit_on_open: bool,
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            commit_nudge_on_close: false,
            block_exit_on_open: true,
        }
    }
}

/// Factory configuration for multi-agent sessions (native TUI)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationConfig {
    /// Default number of worker agents (default: 0 for supervisor-only startup)
    #[serde(default = "default_orchestration_pane_count")]
    pub default_workers: u8,

    /// Auto-prompting configuration for factory events
    #[serde(default)]
    pub auto_prompt: AutoPromptConfig,
}

fn default_orchestration_pane_count() -> u8 {
    0 // Supervisor-only by default for EPIC planning
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            default_workers: default_orchestration_pane_count(),
            auto_prompt: AutoPromptConfig::default(),
        }
    }
}

/// Factory mode configuration for supervisor task assignment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryConfig {
    /// Warn when assigning tasks to workers with stale worktrees
    #[serde(default = "default_true")]
    pub warn_stale_assignment: bool,

    /// Block task assignment to workers with stale worktrees (if commits behind >= threshold)
    #[serde(default)]
    pub block_stale_assignment: bool,

    /// Number of commits behind the sync target before considering a worktree stale
    #[serde(default = "default_stale_threshold")]
    pub stale_threshold_commits: u32,
}

fn default_stale_threshold() -> u32 {
    1
}

impl Default for FactoryConfig {
    fn default() -> Self {
        Self {
            warn_stale_assignment: true,
            block_stale_assignment: true,
            stale_threshold_commits: default_stale_threshold(),
        }
    }
}

/// Code indexing configuration for background code indexing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeConfig {
    /// Whether background code indexing is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Paths to watch for code changes (relative to project root)
    #[serde(default = "default_code_watch_paths")]
    pub watch_paths: Vec<String>,

    /// Glob patterns for directories/files to exclude from indexing
    #[serde(default = "default_code_exclude_patterns")]
    pub exclude_patterns: Vec<String>,

    /// File extensions to index (without leading dot)
    #[serde(default = "default_code_extensions")]
    pub extensions: Vec<String>,

    /// How often to run full code indexing (seconds)
    #[serde(default = "default_code_index_interval")]
    pub index_interval_secs: u64,

    /// Debounce time for file watcher events (milliseconds)
    #[serde(default = "default_code_debounce")]
    pub debounce_ms: u64,
}

fn default_code_watch_paths() -> Vec<String> {
    vec!["src".into(), "lib".into(), "crates".into()]
}

fn default_code_exclude_patterns() -> Vec<String> {
    vec![
        "target/**".into(),
        "node_modules/**".into(),
        ".git/**".into(),
        "dist/**".into(),
        "build/**".into(),
        "_build/**".into(),
        "deps/**".into(),
        "vendor/**".into(),
    ]
}

fn default_code_extensions() -> Vec<String> {
    vec![
        "rs".into(),
        "ts".into(),
        "tsx".into(),
        "js".into(),
        "jsx".into(),
        "py".into(),
        "go".into(),
        "ex".into(),
        "exs".into(),
        "rb".into(),
        "java".into(),
        "kt".into(),
        "swift".into(),
    ]
}

fn default_code_index_interval() -> u64 {
    60 // 1 minute
}

fn default_code_debounce() -> u64 {
    500 // 500ms
}

impl Default for CodeConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Opt-in for CPU-intensive feature
            watch_paths: default_code_watch_paths(),
            exclude_patterns: default_code_exclude_patterns(),
            extensions: default_code_extensions(),
            index_interval_secs: default_code_index_interval(),
            debounce_ms: default_code_debounce(),
        }
    }
}

/// Notification configuration for TUI alerts and hook notifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    /// Master switch for notifications
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Play terminal bell on new notifications
    #[serde(default = "default_true")]
    pub sound_enabled: bool,

    /// How long to display notifications (seconds)
    #[serde(default = "default_display_duration")]
    pub display_duration_secs: u64,

    /// Maximum notifications to display at once
    #[serde(default = "default_max_visible")]
    pub max_visible: usize,

    /// Task notification settings
    #[serde(default)]
    pub tasks: TaskNotifications,

    /// Entry/memory notification settings
    #[serde(default)]
    pub entries: EntryNotifications,

    /// Rule notification settings
    #[serde(default)]
    pub rules: RuleNotifications,

    /// Skill notification settings
    #[serde(default)]
    pub skills: SkillNotifications,

    // === Hook notification settings (for Notification hook) ===
    /// Notify on permission prompts (Claude needs user approval)
    #[serde(default)]
    pub on_permission_prompt: bool,

    /// Notify when Claude is idle and waiting for input
    #[serde(default)]
    pub on_idle_prompt: bool,

    /// Notify on successful authentication
    #[serde(default)]
    pub on_auth_success: bool,

    /// Optional webhook URL for Slack/Discord integration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
}

/// Task-specific notification settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNotifications {
    /// Notify when a task is created
    #[serde(default = "default_true")]
    pub on_created: bool,

    /// Notify when a task is started
    #[serde(default = "default_true")]
    pub on_started: bool,

    /// Notify when a task is closed
    #[serde(default = "default_true")]
    pub on_closed: bool,

    /// Notify when a task is updated (off by default - too noisy)
    #[serde(default)]
    pub on_updated: bool,
}

impl Default for TaskNotifications {
    fn default() -> Self {
        Self {
            on_created: true,
            on_started: true,
            on_closed: true,
            on_updated: false,
        }
    }
}

/// Entry/memory notification settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryNotifications {
    /// Notify when an entry is added
    #[serde(default = "default_true")]
    pub on_added: bool,

    /// Notify when an entry is updated (off by default)
    #[serde(default)]
    pub on_updated: bool,

    /// Notify when an entry is deleted
    #[serde(default = "default_true")]
    pub on_deleted: bool,
}

impl Default for EntryNotifications {
    fn default() -> Self {
        Self {
            on_added: true,
            on_updated: false,
            on_deleted: true,
        }
    }
}

/// Rule notification settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleNotifications {
    /// Notify when a rule is created
    #[serde(default = "default_true")]
    pub on_created: bool,

    /// Notify when a rule is promoted to Proven
    #[serde(default = "default_true")]
    pub on_promoted: bool,

    /// Notify when a rule is demoted (off by default)
    #[serde(default)]
    pub on_demoted: bool,
}

impl Default for RuleNotifications {
    fn default() -> Self {
        Self {
            on_created: true,
            on_promoted: true,
            on_demoted: false,
        }
    }
}

/// Skill notification settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillNotifications {
    /// Notify when a skill is created
    #[serde(default = "default_true")]
    pub on_created: bool,

    /// Notify when a skill is enabled
    #[serde(default = "default_true")]
    pub on_enabled: bool,

    /// Notify when a skill is disabled (off by default)
    #[serde(default)]
    pub on_disabled: bool,
}

impl Default for SkillNotifications {
    fn default() -> Self {
        Self {
            on_created: true,
            on_enabled: true,
            on_disabled: false,
        }
    }
}

fn default_display_duration() -> u64 {
    5 // 5 seconds
}

fn default_max_visible() -> usize {
    3
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sound_enabled: true,
            display_duration_secs: default_display_duration(),
            max_visible: default_max_visible(),
            tasks: TaskNotifications::default(),
            entries: EntryNotifications::default(),
            rules: RuleNotifications::default(),
            skills: SkillNotifications::default(),
            // Hook notification settings (disabled by default)
            on_permission_prompt: false,
            on_idle_prompt: false,
            on_auth_success: false,
            webhook_url: None,
        }
    }
}

/// Cloud sync configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSyncConfig {
    /// Whether auto-sync is enabled (when logged in)
    #[serde(default = "default_true")]
    pub auto_sync: bool,

    /// How often to sync (seconds)
    #[serde(default = "default_cloud_sync_interval")]
    pub interval_secs: u64,

    /// Pull from cloud on MCP server startup
    #[serde(default = "default_true")]
    pub pull_on_start: bool,

    /// Maximum retry attempts for failed syncs
    #[serde(default = "default_max_retries")]
    pub max_retries: i32,
}

fn default_cloud_sync_interval() -> u64 {
    300 // 5 minutes
}

fn default_max_retries() -> i32 {
    5
}

impl Default for CloudSyncConfig {
    fn default() -> Self {
        Self {
            auto_sync: true,
            interval_secs: default_cloud_sync_interval(),
            pull_on_start: true,
            max_retries: default_max_retries(),
        }
    }
}

/// Development mode configuration for tracing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevConfig {
    /// Enable dev mode tracing
    #[serde(default)]
    pub dev_mode: bool,

    /// Trace CLI command executions
    #[serde(default = "default_true")]
    pub trace_commands: bool,

    /// Trace store operations (add, update, delete, get)
    #[serde(default = "default_true")]
    pub trace_store_ops: bool,

    /// Trace Claude API calls with full prompts/responses
    #[serde(default = "default_true")]
    pub trace_claude_api: bool,

    /// Trace hook events
    #[serde(default = "default_true")]
    pub trace_hooks: bool,

    /// Days to retain traces before auto-cleanup
    #[serde(default = "default_trace_retention")]
    pub trace_retention_days: i64,
}

fn default_trace_retention() -> i64 {
    7
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            dev_mode: false,
            trace_commands: true,
            trace_store_ops: true,
            trace_claude_api: true,
            trace_hooks: true,
            trace_retention_days: 7,
        }
    }
}

/// Telemetry configuration for anonymous usage tracking
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Whether telemetry is enabled (default: false, opt-in via CAS_TELEMETRY=1)
    #[serde(default)]
    pub enabled: bool,

    /// Anonymous user ID (generated on first run)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anonymous_id: Option<String>,

    /// Whether user has given consent for telemetry (None = not asked yet)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent_given: Option<bool>,
}

/// LLM configuration for harness and model selection
///
/// Controls which CLI harness (Claude or Codex) is used and which model
/// each harness runs. Per-role overrides allow different configurations
/// for supervisor vs worker agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Which CLI harness to use: "claude" or "codex"
    #[serde(default = "default_harness")]
    pub harness: String,

    /// Model to use within the harness (e.g., "claude-sonnet-4-5-20250929", "gpt-5.3-codex")
    /// If not set, the harness uses its default model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Reasoning effort level: "low", "medium", or "high" (only supported by some models)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,

    /// Override configuration for supervisor agents
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor: Option<LlmRoleConfig>,

    /// Override configuration for worker agents
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<LlmRoleConfig>,
}

/// Per-role LLM overrides (supervisor or worker)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRoleConfig {
    /// Override harness for this role
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,

    /// Override model for this role
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Override reasoning effort for this role
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

fn default_harness() -> String {
    "claude".to_string()
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            harness: default_harness(),
            model: None,
            reasoning_effort: None,
            supervisor: None,
            worker: None,
        }
    }
}

impl LlmConfig {
    /// Resolve the harness for a given role, falling back to the top-level setting.
    pub fn harness_for_role(&self, role: &str) -> &str {
        let role_override = match role {
            "supervisor" => self.supervisor.as_ref().and_then(|r| r.harness.as_deref()),
            "worker" => self.worker.as_ref().and_then(|r| r.harness.as_deref()),
            _ => None,
        };
        role_override.unwrap_or(&self.harness)
    }

    /// Resolve the model for a given role, falling back to the top-level setting.
    pub fn model_for_role(&self, role: &str) -> Option<&str> {
        let role_override = match role {
            "supervisor" => self.supervisor.as_ref().and_then(|r| r.model.as_deref()),
            "worker" => self.worker.as_ref().and_then(|r| r.model.as_deref()),
            _ => None,
        };
        role_override.or(self.model.as_deref())
    }

    /// Resolve the reasoning effort for a given role, falling back to the top-level setting.
    pub fn reasoning_effort_for_role(&self, role: &str) -> Option<&str> {
        let role_override = match role {
            "supervisor" => self
                .supervisor
                .as_ref()
                .and_then(|r| r.reasoning_effort.as_deref()),
            "worker" => self
                .worker
                .as_ref()
                .and_then(|r| r.reasoning_effort.as_deref()),
            _ => None,
        };
        role_override.or(self.reasoning_effort.as_deref())
    }
}

fn default_true() -> bool {
    true
}

fn default_target() -> String {
    ".claude/rules/cas".to_string()
}

fn default_min_helpful() -> i32 {
    1
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            target: ".claude/rules/cas".to_string(),
            min_helpful: 1,
        }
    }
}
