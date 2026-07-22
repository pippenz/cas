use crate::config::*;

impl Config {
    /// Load configuration from .cas directory
    ///
    /// Tries TOML first (config.toml), falls back to YAML (config.yaml),
    /// and auto-migrates YAML to TOML on first load.
    ///
    /// When both files exist, merges any YAML-only settings into the TOML
    /// config (covers the case where something wrote to config.yaml while
    /// config.toml already existed).
    pub fn load(cas_dir: &std::path::Path) -> Result<Self, MemError> {
        let toml_path = cas_dir.join("config.toml");
        let yaml_path = cas_dir.join("config.yaml");

        // Try TOML first (preferred format)
        if toml_path.exists() {
            let content = std::fs::read_to_string(&toml_path)?;
            let mut config: Self = toml::from_str(&content)
                .map_err(|e| MemError::Parse(format!("Failed to parse config.toml: {e}")))?;

            // If YAML also exists, merge any settings that are missing from TOML.
            // This handles the case where something wrote to config.yaml after
            // config.toml was already created (e.g. theme variant).
            if yaml_path.exists() {
                if let Ok(yaml_content) = std::fs::read_to_string(&yaml_path) {
                    if let Ok(yaml_config) = serde_yaml::from_str::<Self>(&yaml_content) {
                        let changed = config.merge_missing(&yaml_config);
                        if changed {
                            // Persist the merged config and clean up stale YAML
                            let _ = config.save_toml(cas_dir);
                        }
                        // Always remove the stale YAML to prevent future confusion
                        let backup_path = cas_dir.join("config.yaml.bak");
                        let _ = std::fs::rename(&yaml_path, &backup_path);
                    }
                }
            }

            return Ok(config);
        }

        // Fall back to YAML and auto-migrate
        if yaml_path.exists() {
            let content = std::fs::read_to_string(&yaml_path)?;
            let config: Self = serde_yaml::from_str(&content)?;

            // Auto-migrate to TOML
            if let Err(e) = config.save_toml(cas_dir) {
                eprintln!("Warning: Failed to migrate config to TOML: {e}");
            } else {
                // Rename old YAML to backup
                let backup_path = cas_dir.join("config.yaml.bak");
                if let Err(e) = std::fs::rename(&yaml_path, &backup_path) {
                    eprintln!("Warning: Failed to backup config.yaml: {e}");
                }
            }

            return Ok(config);
        }

        Ok(Self::default())
    }

    /// Load project config with host-level `~/.cas/config.toml` defaults.
    ///
    /// Project config wins when it sets a section; host config fills missing
    /// top-level sections. This is used for machine-scoped settings that need
    /// to be visible from hooks running inside arbitrary project worktrees.
    pub fn load_with_host_defaults(cas_dir: &std::path::Path) -> Result<Self, MemError> {
        let mut config = Self::load(cas_dir)?;
        let host_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".cas");

        if host_dir != cas_dir {
            let host_config = Self::load(&host_dir).unwrap_or_default();
            config.merge_missing(&host_config);
        }

        Ok(config)
    }

    /// Save configuration to .cas directory as TOML (preferred format)
    pub fn save(&self, cas_dir: &std::path::Path) -> Result<(), MemError> {
        self.save_toml(cas_dir)
    }

    /// Save configuration as TOML
    pub fn save_toml(&self, cas_dir: &std::path::Path) -> Result<(), MemError> {
        let config_path = cas_dir.join("config.toml");
        let content = toml::to_string_pretty(self)
            .map_err(|e| MemError::Parse(format!("Failed to serialize config to TOML: {e}")))?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    /// Save configuration as YAML (legacy format)
    #[deprecated(note = "YAML config is legacy; use config.toml")]
    pub fn save_yaml(&self, cas_dir: &std::path::Path) -> Result<(), MemError> {
        let _ = cas_dir;
        Err(MemError::Parse(
            "YAML config is deprecated; use config.toml".to_string(),
        ))
    }

    /// Get path to config file (TOML preferred, YAML fallback)
    pub fn config_path(cas_dir: &std::path::Path) -> std::path::PathBuf {
        cas_dir.join("config.toml")
    }

    /// Check if sync is disabled via environment variable
    pub fn is_sync_disabled() -> bool {
        std::env::var("MEM_SYNC_DISABLED")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false)
    }

    /// Resolve `.cas/config.toml` `[factory] epic_base_branch` for the repo
    /// at `repo_root`, or `None` when unset / the config can't be read.
    ///
    /// Shared by epic-branch auto-creation and worker-spawn base resolution
    /// (cas-b082) so both paths agree on the configured trunk before
    /// falling back to `GitOperations::detect_default_branch()`.
    pub fn configured_epic_base_branch(repo_root: &std::path::Path) -> Option<String> {
        Self::load(&repo_root.join(".cas"))
            .ok()?
            .factory()
            .epic_base_branch
    }
}
