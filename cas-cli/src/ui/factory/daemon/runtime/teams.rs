//! Native Agent Teams integration for factory daemon.
//!
//! Manages Claude Code's native Agent Teams file structure:
//! - `~/.claude/teams/{team-name}/config.json` — team member registry
//! - `~/.claude/teams/{team-name}/inboxes/{agent-name}.json` — per-agent inbox files
//!
//! This replaces the old prompt_queue + mux.inject (PTY stdin injection) transport
//! with native Teams mailbox writes that Claude Code polls internally.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Colors assigned to team members (matches Claude Code's palette).
const AGENT_COLORS: &[&str] = &["green", "blue", "yellow", "cyan", "magenta", "red", "white"];

/// The director agent name registered in the team config.
/// The daemon uses this identity when writing system/auto-prompt messages
/// to agent inboxes so that Claude Code recognizes the sender as a valid
/// team member.
pub const DIRECTOR_AGENT_NAME: &str = "director";

/// A single message in a Teams inbox file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxMessage {
    pub from: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub timestamp: String,
    pub color: String,
    pub read: bool,
}

/// Team member entry in config.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    pub agent_id: String,
    pub name: String,
    pub agent_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_mode_required: Option<bool>,
    pub joined_at: u64,
    pub tmux_pane_id: String,
    pub cwd: String,
    #[serde(default)]
    pub subscriptions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_type: Option<String>,
}

/// Team config.json structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamConfig {
    pub name: String,
    pub description: String,
    pub created_at: u64,
    pub lead_agent_id: String,
    pub lead_session_id: String,
    pub members: Vec<TeamMember>,
}

/// Manages the native Agent Teams file structure for a factory session.
pub struct TeamsManager {
    team_name: String,
    teams_dir: PathBuf,
    inboxes_dir: PathBuf,
}

impl TeamsManager {
    /// Create a new TeamsManager for the given factory session.
    ///
    /// The team name is derived from the session name.
    /// Files are stored at `~/.claude/teams/{team-name}/`.
    pub fn new(session_name: &str) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let teams_dir = home.join(".claude").join("teams").join(session_name);
        let inboxes_dir = teams_dir.join("inboxes");

        Self {
            team_name: session_name.to_string(),
            teams_dir,
            inboxes_dir,
        }
    }

    /// Get the team name.
    pub fn team_name(&self) -> &str {
        &self.team_name
    }

    /// Format an agent ID: `{name}@{team-name}`.
    pub fn agent_id_for(&self, name: &str) -> String {
        format!("{}@{}", name, self.team_name)
    }

    /// Build a teams_configs HashMap for MuxConfig before agents are spawned.
    ///
    /// This is a static method because it's called before TeamsManager is fully
    /// initialized (before `init_team_config`). It constructs the CLI flags map
    /// that Mux::factory() uses when spawning agent PTYs.
    /// Returns `(configs_map, lead_session_id)`.
    pub fn build_configs_for_mux(
        session_name: &str,
        supervisor_name: &str,
        worker_names: &[String],
    ) -> (
        std::collections::HashMap<String, cas_mux::TeamsSpawnConfig>,
        String,
    ) {
        let mut configs = std::collections::HashMap::new();
        let lead_session_id = uuid::Uuid::new_v4().to_string();

        // Supervisor settings path — auto-allow Write/Edit/Bash/NotebookEdit so
        // the supervisor's tool calls don't get forwarded to itself via team
        // permission routing (self-leadership deadlock). Workers leave this as
        // None and retain normal routing. The file is written by
        // `init_team_config` below; the path is computed here so the CLI flag
        // is in place before the PTY spawn.
        let supervisor_settings_path =
            Self::supervisor_settings_path_for(session_name).to_string_lossy().to_string();

        // Supervisor — keyed by pane name for PTY lookup, but agent_name is
        // always "supervisor" so Claude identifies as "supervisor" in the team.
        configs.insert(
            supervisor_name.to_string(),
            cas_mux::TeamsSpawnConfig {
                team_name: session_name.to_string(),
                agent_id: format!("supervisor@{}", session_name),
                agent_name: "supervisor".to_string(),
                agent_color: "green".to_string(),
                agent_type: "team-lead".to_string(),
                parent_session_id: None,
                lead_session_id: Some(lead_session_id.clone()),
                settings_path: Some(supervisor_settings_path),
            },
        );

        // Workers
        for (i, name) in worker_names.iter().enumerate() {
            configs.insert(
                name.clone(),
                cas_mux::TeamsSpawnConfig {
                    team_name: session_name.to_string(),
                    agent_id: format!("{}@{}", name, session_name),
                    agent_name: name.clone(),
                    agent_color: Self::color_for_index(i).to_string(),
                    agent_type: "general-purpose".to_string(),
                    parent_session_id: Some(lead_session_id.clone()),
                    lead_session_id: None,
                    settings_path: None,
                },
            );
        }

        (configs, lead_session_id)
    }

    /// Compute the on-disk path of the supervisor-only settings file for a
    /// given session name. The file lives alongside `config.json` under
    /// `~/.claude/teams/{session}/supervisor-settings.json` and is written by
    /// [`init_team_config`]. See [`supervisor_settings_contents`] for the
    /// allowlist shape.
    pub fn supervisor_settings_path_for(session_name: &str) -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".claude")
            .join("teams")
            .join(session_name)
            .join("supervisor-settings.json")
    }

    /// The JSON body of the supervisor settings file — a Claude Code
    /// `permissions.allow` list that auto-approves the four tool families
    /// whose approvals would otherwise route back to the supervisor itself.
    ///
    /// Kept tight on purpose: no MCP tools, no network, no shell glob expansion
    /// beyond the base tool name. The deadlock only fires for tools that are
    /// not otherwise auto-allowed, and the factory supervisor already runs
    /// with `--dangerously-skip-permissions`, so this list is the minimum set
    /// observed to produce the routing-deadlock symptom.
    pub fn supervisor_settings_contents() -> serde_json::Value {
        serde_json::json!({
            "permissions": {
                "allow": [
                    "Write",
                    "Edit",
                    "Bash",
                    "NotebookEdit"
                ]
            }
        })
    }

    /// Assign a color to an agent based on its index in the team.
    pub fn color_for_index(index: usize) -> &'static str {
        AGENT_COLORS[index % AGENT_COLORS.len()]
    }

    /// Initialize the team directory and write config.json with supervisor + initial workers.
    ///
    /// `worker_cwds` maps worker names to their actual working directories (worktree paths
    /// when worktrees are enabled). Workers not in the map use `project_cwd` as fallback.
    pub fn init_team_config(
        &self,
        worker_names: &[String],
        project_cwd: &std::path::Path,
        worker_cwds: &std::collections::HashMap<String, std::path::PathBuf>,
        lead_session_id: &str,
    ) -> anyhow::Result<()> {
        // Create directories
        std::fs::create_dir_all(&self.inboxes_dir)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let project_cwd_str = project_cwd.to_string_lossy().to_string();

        let model = Some("claude-opus-4-6".to_string());

        // Supervisor is the team lead but also a teammate so it polls its inbox.
        // Always registered as "supervisor" regardless of the generated pane name.
        let mut members = vec![TeamMember {
            agent_id: self.agent_id_for("supervisor"),
            name: "supervisor".to_string(),
            agent_type: "team-lead".to_string(),
            model: model.clone(),
            prompt: None,
            color: Some("green".to_string()),
            plan_mode_required: None,
            joined_at: now,
            tmux_pane_id: "tmux".to_string(),
            cwd: project_cwd_str.clone(),
            subscriptions: Vec::new(),
            backend_type: Some("tmux".to_string()),
        }];

        // Director is the daemon's identity for system/auto-prompt messages.
        // Registered as a team member so Claude Code accepts messages from it.
        members.push(TeamMember {
            agent_id: self.agent_id_for(DIRECTOR_AGENT_NAME),
            name: DIRECTOR_AGENT_NAME.to_string(),
            agent_type: "director".to_string(),
            model: model.clone(),
            prompt: None,
            color: Some("white".to_string()),
            plan_mode_required: None,
            joined_at: now,
            tmux_pane_id: "tmux".to_string(),
            cwd: project_cwd_str.clone(),
            subscriptions: Vec::new(),
            backend_type: Some("tmux".to_string()),
        });

        // Add workers (each may have its own worktree path)
        for (i, worker_name) in worker_names.iter().enumerate() {
            let worker_cwd = worker_cwds
                .get(worker_name)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| project_cwd_str.clone());

            members.push(TeamMember {
                agent_id: self.agent_id_for(worker_name),
                name: worker_name.clone(),
                agent_type: "general-purpose".to_string(),
                model: model.clone(),
                prompt: None,
                color: Some(Self::color_for_index(i).to_string()),
                plan_mode_required: Some(false),
                joined_at: now,
                tmux_pane_id: "tmux".to_string(),
                cwd: worker_cwd,
                subscriptions: Vec::new(),
                backend_type: Some("tmux".to_string()),
            });
        }

        let config = TeamConfig {
            name: self.team_name.clone(),
            description: format!("CAS factory session {}", self.team_name),
            created_at: now,
            lead_agent_id: self.agent_id_for("supervisor"),
            lead_session_id: lead_session_id.to_string(),
            members,
        };

        let config_path = self.teams_dir.join("config.json");
        let json = serde_json::to_string_pretty(&config)?;
        std::fs::write(&config_path, json)?;

        // Write the supervisor-only settings file. This gets picked up via
        // `--settings <path>` on the supervisor's `claude` invocation (wired
        // through `TeamsSpawnConfig::settings_path`) and auto-allows the four
        // tool families that would otherwise hang — Write/Edit/Bash/NotebookEdit.
        // Without it, the supervisor's own tool use hits team permission
        // routing, gets forwarded to the team leader (itself), and deadlocks.
        self.write_supervisor_settings()?;

        // Create empty inbox files for all agents
        self.ensure_inbox("supervisor")?;
        self.ensure_inbox(DIRECTOR_AGENT_NAME)?;
        for worker_name in worker_names {
            self.ensure_inbox(worker_name)?;
        }

        tracing::info!(
            "Initialized Teams config at {:?} with {} members",
            config_path,
            1 + worker_names.len()
        );

        Ok(())
    }

    /// Write `supervisor-settings.json` in the team directory. Safe to call
    /// multiple times; the content is fixed so repeated writes are idempotent.
    pub fn write_supervisor_settings(&self) -> anyhow::Result<()> {
        let path = self.teams_dir.join("supervisor-settings.json");
        let body = serde_json::to_string_pretty(&Self::supervisor_settings_contents())?;
        std::fs::write(&path, body)?;
        tracing::info!("Wrote supervisor settings at {:?}", path);
        Ok(())
    }

    /// Add a new member to the team (e.g., when a worker is spawned dynamically).
    pub fn add_member(
        &self,
        name: &str,
        cwd: &std::path::Path,
        color_index: usize,
    ) -> anyhow::Result<()> {
        let config_path = self.teams_dir.join("config.json");
        let json = std::fs::read_to_string(&config_path)?;
        let mut config: TeamConfig = serde_json::from_str(&json)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        config.members.push(TeamMember {
            agent_id: self.agent_id_for(name),
            name: name.to_string(),
            agent_type: "general-purpose".to_string(),
            model: Some("claude-opus-4-6".to_string()),
            prompt: None,
            color: Some(Self::color_for_index(color_index).to_string()),
            plan_mode_required: Some(false),
            joined_at: now,
            tmux_pane_id: "tmux".to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            subscriptions: Vec::new(),
            backend_type: Some("tmux".to_string()),
        });

        let json = serde_json::to_string_pretty(&config)?;
        std::fs::write(&config_path, json)?;

        self.ensure_inbox(name)?;

        tracing::info!("Added team member '{}' to {}", name, self.team_name);
        Ok(())
    }

    /// Remove a member from the team (e.g., when a worker is shut down).
    pub fn remove_member(&self, name: &str) -> anyhow::Result<()> {
        let config_path = self.teams_dir.join("config.json");
        let json = std::fs::read_to_string(&config_path)?;
        let mut config: TeamConfig = serde_json::from_str(&json)?;

        config.members.retain(|m| m.name != name);

        let json = serde_json::to_string_pretty(&config)?;
        std::fs::write(&config_path, json)?;

        // Remove inbox file
        let inbox_path = self.inboxes_dir.join(format!("{}.json", name));
        let _ = std::fs::remove_file(&inbox_path);

        tracing::info!("Removed team member '{}' from {}", name, self.team_name);
        Ok(())
    }

    /// Write a message to a target agent's inbox file.
    ///
    /// Uses file locking to prevent corruption when multiple writers
    /// (daemon + agents) access the same inbox concurrently.
    pub fn write_to_inbox(
        &self,
        target: &str,
        from: &str,
        message: &str,
        summary: Option<&str>,
        color: Option<&str>,
    ) -> anyhow::Result<()> {
        let inbox_path = self.inboxes_dir.join(format!("{}.json", target));

        // Ensure inbox file exists
        if !inbox_path.exists() {
            std::fs::write(&inbox_path, "[]")?;
        }

        // Use file locking for safe concurrent access
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&inbox_path)?;

        // Acquire exclusive lock
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let ret = unsafe { libc::flock(fd, libc::LOCK_EX) };
        if ret != 0 {
            anyhow::bail!(
                "Failed to lock inbox file {:?}: {}",
                inbox_path,
                std::io::Error::last_os_error()
            );
        }

        // Read existing messages
        let mut messages: Vec<InboxMessage> = {
            let content = std::fs::read_to_string(&inbox_path).unwrap_or_else(|_| "[]".to_string());
            serde_json::from_str(&content).unwrap_or_default()
        };

        // Append new message
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let resolved_color = color.unwrap_or("green").to_string();

        // Always set summary — native Claude Code expects it.
        // Fall back to the message text when no explicit summary is provided.
        let resolved_summary = summary.unwrap_or(message).to_string();

        messages.push(InboxMessage {
            from: from.to_string(),
            text: message.to_string(),
            summary: Some(resolved_summary),
            timestamp: now,
            color: resolved_color,
            read: false,
        });

        // Write back
        let json = serde_json::to_string_pretty(&messages)?;
        std::fs::write(&inbox_path, json)?;

        // Release lock (automatic on drop, but be explicit)
        unsafe { libc::flock(fd, libc::LOCK_UN) };

        tracing::debug!("Wrote message to inbox: {} -> {}", from, target);

        Ok(())
    }

    /// Ensure an inbox file exists for the given agent.
    fn ensure_inbox(&self, name: &str) -> anyhow::Result<()> {
        let inbox_path = self.inboxes_dir.join(format!("{}.json", name));
        if !inbox_path.exists() {
            std::fs::write(&inbox_path, "[]")?;
        }
        Ok(())
    }

    /// Clean up the team directory on shutdown.
    pub fn cleanup(&self) {
        if self.teams_dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&self.teams_dir) {
                tracing::warn!("Failed to clean up teams dir {:?}: {}", self.teams_dir, e);
            } else {
                tracing::info!("Cleaned up teams dir {:?}", self.teams_dir);
            }
        }
    }

    /// Remove orphaned team directories whose daemon is no longer running.
    ///
    /// Scans `~/.claude/teams/` for directories and checks if the corresponding
    /// factory daemon socket (`~/.cas/factory-{name}.sock`) still exists. If the
    /// socket is gone, the daemon crashed without cleaning up and the team
    /// directory is safe to remove.
    ///
    /// Called once at daemon startup to clean up after previous crashes.
    pub fn cleanup_orphans() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let teams_root = home.join(".claude").join("teams");

        let entries = match std::fs::read_dir(&teams_root) {
            Ok(entries) => entries,
            Err(_) => return, // No teams directory at all
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            // Check if the factory daemon socket still exists
            let sock_path = home.join(".cas").join(format!("factory-{dir_name}.sock"));

            if !sock_path.exists() {
                tracing::info!(
                    "Removing orphaned teams directory {:?} (no daemon socket)",
                    path
                );
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    tracing::warn!("Failed to remove orphaned teams dir {:?}: {}", path, e);
                }
            }
        }
    }

    /// Build a `cas_mux::TeamsSpawnConfig` for spawning a new agent with native teams flags.
    ///
    /// This constructor is used for dynamically-added workers (agents added
    /// after the initial `init_team_config` call). It intentionally leaves
    /// `settings_path` unset — the supervisor-only settings file is for the
    /// team lead's self-routing deadlock and is not relevant to workers.
    pub fn spawn_config_for(
        &self,
        name: &str,
        agent_type: &str,
        color: &str,
        parent_session_id: Option<&str>,
    ) -> cas_mux::TeamsSpawnConfig {
        cas_mux::TeamsSpawnConfig {
            team_name: self.team_name.clone(),
            agent_id: self.agent_id_for(name),
            agent_name: name.to_string(),
            agent_color: color.to_string(),
            agent_type: agent_type.to_string(),
            parent_session_id: parent_session_id.map(|s| s.to_string()),
            lead_session_id: None,
            settings_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point the manager at a temp directory instead of `~/.claude/teams/...`
    /// so the test doesn't collide with real factory sessions. We keep the
    /// production constructor in place and just override the internal paths;
    /// that also exercises the real file layout the supervisor CLI sees.
    fn manager_in(tmp: &std::path::Path, name: &str) -> TeamsManager {
        let teams_dir = tmp.join(".claude").join("teams").join(name);
        let inboxes_dir = teams_dir.join("inboxes");
        TeamsManager {
            team_name: name.to_string(),
            teams_dir,
            inboxes_dir,
        }
    }

    #[test]
    fn init_team_config_writes_supervisor_settings_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tm = manager_in(tmp.path(), "deadlock-test-team");
        let (_configs, lead_session_id) =
            TeamsManager::build_configs_for_mux("deadlock-test-team", "supervisor", &[]);

        tm.init_team_config(&[], tmp.path(), &std::collections::HashMap::new(), &lead_session_id)
            .expect("init");

        let settings_path = tm.teams_dir.join("supervisor-settings.json");
        assert!(
            settings_path.exists(),
            "supervisor-settings.json should be written next to config.json"
        );

        let body = std::fs::read_to_string(&settings_path).expect("read settings");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        let allow = parsed
            .get("permissions")
            .and_then(|p| p.get("allow"))
            .and_then(|a| a.as_array())
            .expect("permissions.allow array present");

        let names: Vec<&str> = allow.iter().filter_map(|v| v.as_str()).collect();
        // Exactly the four tool families the routing deadlock is observed on.
        // Keeping this assertion tight catches accidental drift toward
        // over-permissioning (which would weaken isolation for the supervisor).
        assert!(names.contains(&"Write"), "allow must include Write");
        assert!(names.contains(&"Edit"), "allow must include Edit");
        assert!(names.contains(&"Bash"), "allow must include Bash");
        assert!(
            names.contains(&"NotebookEdit"),
            "allow must include NotebookEdit"
        );
        assert_eq!(
            names.len(),
            4,
            "allow should be exactly the four deadlock-prone tool families, got {names:?}"
        );
    }

    #[test]
    fn build_configs_for_mux_sets_settings_path_on_supervisor_only() {
        let (configs, _lead_session_id) = TeamsManager::build_configs_for_mux(
            "routing-test-team",
            "supervisor",
            &["worker-1".to_string(), "worker-2".to_string()],
        );

        let sup = configs.get("supervisor").expect("supervisor config");
        let path = sup
            .settings_path
            .as_ref()
            .expect("supervisor must carry a settings_path so --settings is emitted");
        assert!(
            path.ends_with("supervisor-settings.json"),
            "settings_path should point at supervisor-settings.json, got {path}"
        );
        assert!(
            path.contains("routing-test-team"),
            "settings_path should live under the session's team dir, got {path}"
        );

        for worker in ["worker-1", "worker-2"] {
            let w = configs.get(worker).expect("worker config");
            assert!(
                w.settings_path.is_none(),
                "worker {worker} must not get the supervisor allowlist"
            );
        }
    }
}
