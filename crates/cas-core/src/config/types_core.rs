use serde::{Deserialize, Serialize};

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
/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    /// Whether MCP mode is enabled (set during init)
    #[serde(default)]
    pub enabled: bool,
}
/// Embedding daemon configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Whether background embedding generation is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Embedding model to use (Hugging Face model ID)
    #[serde(default = "default_embedding_model")]
    pub model: String,

    /// Reranker model to use (Hugging Face model ID), None to disable reranking
    #[serde(default = "default_embedding_reranker")]
    pub reranker: Option<String>,

    /// Batch size for embedding generation (GPU efficiency)
    #[serde(default = "default_embedding_batch_size")]
    pub batch_size: usize,

    /// Maximum embeddings to generate per daemon run
    #[serde(default = "default_embedding_max_per_run")]
    pub max_per_run: usize,

    /// Interval between embedding runs (seconds)
    #[serde(default = "default_embedding_interval")]
    pub interval_secs: u64,
}

// Note: Local embeddings are deprecated. These defaults are kept for backward compatibility.
// Semantic search is now cloud-only.
fn default_embedding_model() -> String {
    "Qwen/Qwen3-Embedding-0.6B".to_string()
}

fn default_embedding_reranker() -> Option<String> {
    Some("Qwen/Qwen3-Reranker-0.6B".to_string())
}

fn default_embedding_batch_size() -> usize {
    16
}

fn default_embedding_max_per_run() -> usize {
    100
}

fn default_embedding_interval() -> u64 {
    120 // 2 minutes
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: default_embedding_model(),
            reranker: default_embedding_reranker(),
            batch_size: default_embedding_batch_size(),
            max_per_run: default_embedding_max_per_run(),
            interval_secs: default_embedding_interval(),
        }
    }
}
/// Notification configuration for TUI alerts
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
    60 // 1 minute
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
/// Hook configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Whether capture is enabled
    #[serde(default = "default_true")]
    pub capture_enabled: bool,

    /// Tools to capture (empty = use defaults)
    #[serde(default = "default_capture_tools")]
    pub capture_tools: Vec<String>,

    /// Whether to inject context at session start
    #[serde(default = "default_true")]
    pub inject_context: bool,

    /// Maximum entries to include in context
    #[serde(default = "default_context_limit")]
    pub context_limit: usize,

    /// Whether to generate AI summaries at session end
    #[serde(default)]
    pub generate_summaries: bool,

    /// Token budget for context injection (0 = unlimited)
    #[serde(default = "default_token_budget")]
    pub token_budget: usize,

    /// Use AI to prioritize context items (requires claude CLI)
    #[serde(default)]
    pub ai_context: bool,

    /// Model to use for AI context prioritization
    #[serde(default = "default_ai_model")]
    pub ai_model: String,

    /// Plan mode specific configuration
    #[serde(default)]
    pub plan_mode: PlanModeConfig,

    /// Minimal start mode - only inject blocked tasks and pinned memories
    /// Following context engineering principle: "start with minimal prompts"
    #[serde(default)]
    pub minimal_start: bool,
}
/// Plan mode specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanModeConfig {
    /// Whether to use plan-aware context in plan mode
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Token budget for plan mode (typically higher than execution)
    #[serde(default = "default_plan_token_budget")]
    pub token_budget: usize,

    /// Maximum tasks to show in plan context
    #[serde(default = "default_plan_task_limit")]
    pub task_limit: usize,

    /// Include dependency trees in plan context
    #[serde(default = "default_true")]
    pub show_dependencies: bool,

    /// Include closed tasks for reference
    #[serde(default)]
    pub include_closed: bool,

    /// Search for related memories semantically
    #[serde(default = "default_true")]
    pub semantic_search: bool,
}

fn default_plan_token_budget() -> usize {
    8000 // Higher budget for planning
}

fn default_plan_task_limit() -> usize {
    15
}

impl Default for PlanModeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            token_budget: default_plan_token_budget(),
            task_limit: default_plan_task_limit(),
            show_dependencies: true,
            include_closed: false,
            semantic_search: true,
        }
    }
}

fn default_ai_model() -> String {
    "claude-haiku-4-5".to_string()
}

fn default_capture_tools() -> Vec<String> {
    vec!["Write".to_string(), "Edit".to_string(), "Bash".to_string()]
}

fn default_context_limit() -> usize {
    5
}

fn default_token_budget() -> usize {
    4000 // Default ~4k tokens for context injection
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            capture_enabled: true,
            capture_tools: default_capture_tools(),
            inject_context: true,
            context_limit: 5,
            generate_summaries: false, // Disabled by default (requires ai-extraction feature)
            token_budget: default_token_budget(),
            ai_context: false, // Disabled by default (uses heuristic prioritization)
            ai_model: default_ai_model(),
            plan_mode: PlanModeConfig::default(),
            minimal_start: false, // Disabled by default for backward compatibility
        }
    }
}
/// Worktree configuration for automatic git worktree management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreesConfig {
    /// Whether automatic worktree creation is enabled on task start
    #[serde(default)]
    pub enabled: bool,

    /// Base directory for worktrees (relative to repo root's parent)
    /// Supports {project} placeholder for project name
    #[serde(default = "default_worktree_base_path")]
    pub base_path: String,

    /// Prefix for branch names (e.g., "cas/")
    #[serde(default = "default_worktree_branch_prefix")]
    pub branch_prefix: String,

    /// Auto-merge branch on task close (if no conflicts)
    #[serde(default)]
    pub auto_merge: bool,

    /// Auto-cleanup worktree directory on task close
    #[serde(default = "default_true")]
    pub cleanup_on_close: bool,

    /// Promote entries with positive feedback to global scope on merge
    #[serde(default = "default_true")]
    pub promote_entries_on_merge: bool,

    /// Require worktree merge before epic close (triggers worktree-merger jail)
    /// When true (default), closing an epic with an active worktree requires
    /// spawning the worktree-merger agent first. Set to false to allow closing
    /// epics without merging their worktrees.
    #[serde(default = "default_true")]
    pub require_merge_on_epic_close: bool,
}

fn default_worktree_base_path() -> String {
    "{project}/.cas/worktrees".to_string()
}

fn default_worktree_branch_prefix() -> String {
    "cas/".to_string()
}

impl Default for WorktreesConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default for safety
            base_path: default_worktree_base_path(),
            branch_prefix: default_worktree_branch_prefix(),
            auto_merge: false,
            cleanup_on_close: true,
            promote_entries_on_merge: true,
            require_merge_on_epic_close: true, // Require merge by default
        }
    }
}
/// Factory mode configuration for multi-agent coordination
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
    10
}

impl Default for FactoryConfig {
    fn default() -> Self {
        Self {
            warn_stale_assignment: true,
            block_stale_assignment: false,
            stale_threshold_commits: default_stale_threshold(),
        }
    }
}
/// Verification configuration for task quality gates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// Whether verification is enabled on task close
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Model to use for verification (Haiku recommended for speed)
    #[serde(default = "default_verification_model")]
    pub model: String,

    /// Allow force bypass of verification (--force flag)
    #[serde(default)]
    pub force_bypass_allowed: bool,

    /// Maximum time for verification (seconds)
    #[serde(default = "default_verification_timeout")]
    pub timeout_secs: u64,

    /// Pattern detection configuration
    #[serde(default)]
    pub patterns: VerificationPatterns,
}
/// Patterns to detect during verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationPatterns {
    /// Detect TODO/FIXME comments
    #[serde(default = "default_true")]
    pub todo_comments: bool,

    /// Detect temporal shortcuts ("for now", "temporarily", etc.)
    #[serde(default = "default_true")]
    pub temporal_shortcuts: bool,

    /// Detect stub implementations
    #[serde(default = "default_true")]
    pub stub_implementations: bool,

    /// Detect empty catch/error blocks
    #[serde(default = "default_true")]
    pub empty_error_handlers: bool,

    /// Custom patterns to detect (regex)
    #[serde(default)]
    pub custom: Vec<String>,
}

fn default_verification_model() -> String {
    "claude-haiku-4-5".to_string()
}

fn default_verification_timeout() -> u64 {
    120 // 2 minutes
}

impl Default for VerificationPatterns {
    fn default() -> Self {
        Self {
            todo_comments: true,
            temporal_shortcuts: true,
            stub_implementations: true,
            empty_error_handlers: true,
            custom: vec![],
        }
    }
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: default_verification_model(),
            force_bypass_allowed: false,
            timeout_secs: default_verification_timeout(),
            patterns: VerificationPatterns::default(),
        }
    }
}
/// Agent configuration for multi-agent coordination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Human-readable name for this agent
    #[serde(default = "default_agent_name")]
    pub name: String,

    /// Maximum concurrent tasks this agent can claim
    #[serde(default = "default_max_concurrent_tasks")]
    pub max_concurrent_tasks: u32,

    /// Capabilities this agent advertises
    #[serde(default)]
    pub capabilities: Vec<String>,
}

fn default_agent_name() -> String {
    std::env::var("CAS_AGENT_NAME").unwrap_or_else(|_| "Claude".to_string())
}

fn default_max_concurrent_tasks() -> u32 {
    std::env::var("CAS_AGENT_MAX_TASKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: default_agent_name(),
            max_concurrent_tasks: default_max_concurrent_tasks(),
            capabilities: vec!["coding".to_string()],
        }
    }
}
/// Coordination mode for multi-agent setup
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CoordinationMode {
    /// Local SQLite-based coordination (single machine)
    Local,
    /// Cloud PostgreSQL-based coordination (distributed)
    Cloud,
}

impl Default for CoordinationMode {
    fn default() -> Self {
        match std::env::var("CAS_COORDINATION_MODE").as_deref() {
            Ok("cloud") => Self::Cloud,
            _ => Self::Local,
        }
    }
}
/// Coordination configuration for multi-agent mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinationConfig {
    /// Coordination mode: local (SQLite) or cloud (PostgreSQL)
    #[serde(default)]
    pub mode: CoordinationMode,

    /// Cloud coordination URL (only used when mode is Cloud)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_url: Option<String>,
}

impl Default for CoordinationConfig {
    fn default() -> Self {
        Self {
            mode: CoordinationMode::default(),
            cloud_url: std::env::var("CAS_CLOUD_URL").ok(),
        }
    }
}
/// Lease configuration for task claiming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseConfig {
    /// Default lease duration in minutes
    #[serde(default = "default_lease_duration")]
    pub default_duration_mins: u32,

    /// Maximum lease duration in minutes
    #[serde(default = "default_max_lease_duration")]
    pub max_duration_mins: u32,

    /// Heartbeat interval in seconds
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u32,

    /// Grace period after expiry before reclaiming (seconds)
    #[serde(default = "default_expiry_grace")]
    pub expiry_grace_secs: u32,
}

fn default_lease_duration() -> u32 {
    std::env::var("CAS_LEASE_DURATION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
}

fn default_max_lease_duration() -> u32 {
    240 // 4 hours
}

fn default_heartbeat_interval() -> u32 {
    300 // 5 minutes
}

fn default_expiry_grace() -> u32 {
    120 // 2 minutes
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            default_duration_mins: default_lease_duration(),
            max_duration_mins: default_max_lease_duration(),
            heartbeat_interval_secs: default_heartbeat_interval(),
            expiry_grace_secs: default_expiry_grace(),
        }
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
