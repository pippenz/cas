//! Logging infrastructure for CAS
//!
//! Provides structured logging using tracing-subscriber with multiple layers:
//! - Console layer: only when --verbose, writes to stderr
//! - File layer: always on, writes to .cas/logs/
//! - EnvFilter: respects RUST_LOG env var

use std::fs::{self, File};
use std::io;
use std::path::Path;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

/// Initialize the logging system
///
/// Call this early in main() before any other code runs.
///
/// # Arguments
/// * `cas_root` - Path to .cas directory (if available)
/// * `verbose` - Whether --verbose flag was passed
/// * `config` - Logging configuration from config file
pub fn init(cas_root: Option<&Path>, verbose: bool, config: &LoggingConfig) -> io::Result<()> {
    // Build the env filter
    // Priority: RUST_LOG env var > config level > default (info)
    let default_level = config.level.as_str();
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    // Console layer - only when verbose, writes to stderr
    let console_layer = if verbose {
        Some(
            fmt::layer()
                .with_writer(io::stderr)
                .with_ansi(true)
                .with_target(true)
                .with_level(true)
                .with_filter(env_filter.clone()),
        )
    } else {
        None
    };

    // File layer - always on if logging is enabled and cas_root exists
    let file_layer = if config.enabled {
        if let Some(root) = cas_root {
            let log_dir = root.join(&config.log_dir);
            fs::create_dir_all(&log_dir)?;

            // Create daily rotating log file
            let log_file = create_log_file(&log_dir)?;

            Some(
                fmt::layer()
                    .with_writer(log_file)
                    .with_ansi(false)
                    .with_target(true)
                    .with_level(true)
                    .with_filter(EnvFilter::new(default_level)),
            )
        } else {
            None
        }
    } else {
        None
    };

    // Build the subscriber with layers
    let subscriber = tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer);

    // Set as global default (ignore error if already set)
    tracing::subscriber::set_global_default(subscriber).ok();

    Ok(())
}

/// Create a log file with today's date
fn create_log_file(log_dir: &Path) -> io::Result<File> {
    let today = chrono::Local::now().format("%Y-%m-%d");
    let log_path = log_dir.join(format!("cas-{today}.log"));

    File::options().create(true).append(true).open(log_path)
}

/// Clean up old log files based on retention policy
pub fn cleanup_old_logs(log_dir: &Path, retention_days: u32) -> io::Result<usize> {
    let mut removed = 0;
    let cutoff = chrono::Local::now() - chrono::Duration::days(retention_days as i64);

    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Only consider cas-YYYY-MM-DD.log files
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("cas-") && name.ends_with(".log") {
                // Parse date from filename
                let date_str = &name[4..14]; // "YYYY-MM-DD"
                if let Ok(file_date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                    let file_datetime = file_date.and_hms_opt(0, 0, 0).unwrap();
                    if file_datetime < cutoff.naive_local() {
                        fs::remove_file(&path)?;
                        removed += 1;
                    }
                }
            }
        }
    }

    Ok(removed)
}

/// Logging configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoggingConfig {
    /// Whether file-based logging is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Log directory (relative to .cas/)
    #[serde(default = "default_log_dir")]
    pub log_dir: String,

    /// Log level: trace, debug, info, warn, error
    #[serde(default = "default_level")]
    pub level: String,

    /// Days to retain log files
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

fn default_true() -> bool {
    true
}

fn default_log_dir() -> String {
    "logs".to_string()
}

fn default_level() -> String {
    "info".to_string()
}

fn default_retention_days() -> u32 {
    7
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            log_dir: default_log_dir(),
            level: default_level(),
            retention_days: default_retention_days(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::logging::*;
    use tempfile::tempdir;

    #[test]
    fn test_logging_config_defaults() {
        let config = LoggingConfig::default();
        assert!(config.enabled);
        assert_eq!(config.log_dir, "logs");
        assert_eq!(config.level, "info");
        assert_eq!(config.retention_days, 7);
    }

    #[test]
    fn test_create_log_file() {
        let dir = tempdir().unwrap();
        let file = create_log_file(dir.path()).unwrap();
        drop(file);

        // Verify file was created
        let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);

        let entry = entries[0].as_ref().unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(name.starts_with("cas-"));
        assert!(name.ends_with(".log"));
    }

    #[test]
    fn test_cleanup_old_logs() {
        let dir = tempdir().unwrap();

        // Create some old log files
        let old_date = chrono::Local::now() - chrono::Duration::days(10);
        let old_name = format!("cas-{}.log", old_date.format("%Y-%m-%d"));
        fs::write(dir.path().join(&old_name), "old log").unwrap();

        // Create a recent log file
        let recent_date = chrono::Local::now();
        let recent_name = format!("cas-{}.log", recent_date.format("%Y-%m-%d"));
        fs::write(dir.path().join(&recent_name), "recent log").unwrap();

        // Clean up with 7-day retention
        let removed = cleanup_old_logs(dir.path(), 7).unwrap();
        assert_eq!(removed, 1);

        // Verify only recent file remains
        let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }
}
