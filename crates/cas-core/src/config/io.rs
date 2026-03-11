use crate::config::Config;
use crate::error::CoreError;
use std::path::Path;

impl Config {
    pub fn load(cas_dir: &Path) -> Result<Self, CoreError> {
        let toml_path = cas_dir.join("config.toml");
        let yaml_path = cas_dir.join("config.yaml");

        if toml_path.exists() {
            let content = std::fs::read_to_string(&toml_path)?;
            return toml::from_str(&content).map_err(|e| CoreError::Parse(e.to_string()));
        }

        if yaml_path.exists() {
            let content = std::fs::read_to_string(&yaml_path)?;
            let config: Self =
                serde_yaml::from_str(&content).map_err(|e| CoreError::Parse(e.to_string()))?;

            // Migrate legacy YAML to TOML
            if let Err(e) = config.save(cas_dir) {
                eprintln!("Warning: Failed to migrate config to TOML: {}", e);
            } else {
                let backup_path = cas_dir.join("config.yaml.bak");
                if let Err(e) = std::fs::rename(&yaml_path, &backup_path) {
                    eprintln!("Warning: Failed to backup config.yaml: {}", e);
                }
            }

            return Ok(config);
        }

        Ok(Self::default())
    }

    /// Save configuration to .cas directory
    pub fn save(&self, cas_dir: &Path) -> Result<(), CoreError> {
        let config_path = cas_dir.join("config.toml");
        let content = toml::to_string_pretty(self).map_err(|e| CoreError::Parse(e.to_string()))?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    /// Get path to config file
    pub fn config_path(cas_dir: &Path) -> std::path::PathBuf {
        cas_dir.join("config.toml")
    }

    /// Check if sync is disabled via environment variable
    pub fn is_sync_disabled() -> bool {
        std::env::var("MEM_SYNC_DISABLED")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false)
    }
}
