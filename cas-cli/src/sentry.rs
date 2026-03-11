//! Sentry crash reporting integration
//!
//! Provides automatic panic/crash reporting to Sentry for debugging.
//! Respects user telemetry preferences and strips PII from reports.

use sentry::ClientInitGuard;

/// Get Sentry DSN from environment or use default
fn get_sentry_dsn() -> Option<&'static str> {
    // Allow override via environment variable
    if let Some(dsn) = option_env!("SENTRY_DSN") {
        if !dsn.is_empty() {
            return Some(dsn);
        }
    }
    // Production DSN (set at compile time or via env)
    // Returns None if not configured, which disables Sentry
    option_env!("CAS_SENTRY_DSN")
}

/// Initialize Sentry crash reporting if telemetry is enabled.
///
/// Returns a guard that must be kept alive for the duration of the program.
/// When the guard is dropped, Sentry flushes any pending events.
///
/// # Arguments
/// * `telemetry_enabled` - Whether the user has opted into telemetry
///
/// # Returns
/// * `Some(guard)` if Sentry was initialized
/// * `None` if telemetry is disabled or initialization failed
pub fn init(telemetry_enabled: bool) -> Option<ClientInitGuard> {
    if !telemetry_enabled {
        return None;
    }

    // Get DSN from environment or compiled-in default
    let dsn = get_sentry_dsn()?;

    let guard = sentry::init((
        dsn,
        sentry::ClientOptions {
            release: Some(env!("CARGO_PKG_VERSION").into()),
            environment: Some(
                if cfg!(debug_assertions) {
                    "development"
                } else {
                    "production"
                }
                .into(),
            ),
            // Capture panics automatically
            auto_session_tracking: true,
            // Sample rate for error events (100%)
            sample_rate: 1.0,
            // Strip file paths to protect privacy
            before_send: Some(std::sync::Arc::new(|mut event| {
                // Strip absolute paths from stack traces
                for exc in event.exception.values.iter_mut() {
                    if let Some(ref mut stacktrace) = exc.stacktrace {
                        for frame in stacktrace.frames.iter_mut() {
                            // Replace absolute paths with relative ones
                            if let Some(ref filename) = frame.filename {
                                if filename.contains('/') {
                                    frame.filename =
                                        filename.rsplit('/').next().map(|s| s.to_string());
                                }
                            }
                            // Remove abs_path entirely
                            frame.abs_path = None;
                        }
                    }
                }
                Some(event)
            })),
            ..Default::default()
        },
    ));

    // Set common context
    sentry::configure_scope(|scope| {
        scope.set_tag("os", std::env::consts::OS);
        scope.set_tag("arch", std::env::consts::ARCH);

        // Add session ID if available (from CLAUDE_CODE_SESSION_ID env var)
        if let Ok(session_id) = std::env::var("CLAUDE_CODE_SESSION_ID") {
            scope.set_tag("session_id", &session_id);
        }
    });

    Some(guard)
}

/// Add breadcrumb for command execution (for crash context)
pub fn add_command_breadcrumb(command: &str) {
    sentry::add_breadcrumb(sentry::Breadcrumb {
        ty: "info".into(),
        category: Some("command".into()),
        message: Some(format!("Executing: {command}")),
        level: sentry::Level::Info,
        ..Default::default()
    });
}

/// Check if Sentry crash reporting is enabled.
///
/// Sentry is opt-in: only enabled when CAS_SENTRY=1 env var is set
/// or explicitly enabled in config.
pub fn is_sentry_enabled() -> bool {
    use crate::config::Config;
    use crate::store::find_cas_root;

    // Check environment variable: CAS_SENTRY=1 enables
    if let Ok(val) = std::env::var("CAS_SENTRY") {
        if val == "1" || val.to_lowercase() == "true" {
            return true;
        }
    }

    // Try to load config from project .cas directory
    if let Ok(cas_root) = find_cas_root() {
        if let Ok(config) = Config::load(&cas_root) {
            return config.telemetry.map(|t| t.enabled).unwrap_or(false);
        }
    }

    // Try global config
    if let Some(global_dir) = crate::config::global_cas_dir() {
        if let Ok(config) = Config::load(&global_dir) {
            return config.telemetry.map(|t| t.enabled).unwrap_or(false);
        }
    }

    // Default to disabled
    false
}

#[cfg(test)]
mod tests {
    use crate::sentry::*;

    #[test]
    fn test_telemetry_disabled_returns_none() {
        let guard = init(false);
        assert!(guard.is_none());
    }
}
