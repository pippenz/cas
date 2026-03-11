//! Integration tests for CLI and Desktop authentication
//!
//! Tests that verify:
//! 1. Token format compatibility between CLI and Desktop
//! 2. Shared auth.json storage works correctly
//! 3. Token expiry validation works consistently
//!
//! These tests use a temporary directory to avoid affecting real auth state.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

// =============================================================================
// Types (matching CLI cas-cli/src/auth/mod.rs)
// =============================================================================

/// CLI AuthUser struct (from cas-cli/src/auth/mod.rs)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct CliAuthUser {
    id: String,
    github_id: i64,
    github_username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_url: Option<String>,
}

/// CLI AuthToken struct (from cas-cli/src/auth/mod.rs)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CliAuthToken {
    access_token: String,
    refresh_token: String,
    expires_at: DateTime<Utc>,
    user: CliAuthUser,
}

// =============================================================================
// Types (matching Desktop cas-desktop/src-tauri/src/auth.rs)
// =============================================================================

/// Desktop AuthUser struct (from cas-desktop/src-tauri/src/auth.rs)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct DesktopAuthUser {
    id: String,
    github_id: i64,
    github_username: String,
    email: Option<String>,
    avatar_url: Option<String>,
}

/// Desktop AuthToken struct (from cas-desktop/src-tauri/src/auth.rs)
/// Note: Desktop uses String for expires_at (ISO 8601 format)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopAuthToken {
    access_token: String,
    refresh_token: String,
    expires_at: String,
    user: DesktopAuthUser,
}

// =============================================================================
// Test Helpers
// =============================================================================

fn create_temp_auth_dir() -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let auth_path = temp.path().join("auth.json");
    (temp, auth_path)
}

fn create_cli_test_token() -> CliAuthToken {
    CliAuthToken {
        access_token: "cli_access_token_abc123".to_string(),
        refresh_token: "cli_refresh_token_xyz789".to_string(),
        expires_at: Utc::now() + Duration::hours(1),
        user: CliAuthUser {
            id: "user-uuid-12345".to_string(),
            github_id: 12345678,
            github_username: "testuser".to_string(),
            email: Some("test@example.com".to_string()),
            avatar_url: Some("https://github.com/testuser.png".to_string()),
        },
    }
}

fn create_desktop_test_token() -> DesktopAuthToken {
    DesktopAuthToken {
        access_token: "desktop_access_token_def456".to_string(),
        refresh_token: "desktop_refresh_token_uvw321".to_string(),
        expires_at: (Utc::now() + Duration::hours(1)).to_rfc3339(),
        user: DesktopAuthUser {
            id: "user-uuid-67890".to_string(),
            github_id: 87654321,
            github_username: "desktopuser".to_string(),
            email: Some("desktop@example.com".to_string()),
            avatar_url: Some("https://github.com/desktopuser.png".to_string()),
        },
    }
}

// =============================================================================
// Token Format Compatibility Tests
// =============================================================================

#[test]
fn test_cli_token_can_be_read_by_desktop() {
    // CLI writes a token, Desktop should be able to read it
    let (_temp, auth_path) = create_temp_auth_dir();

    // CLI writes token
    let cli_token = create_cli_test_token();
    let json = serde_json::to_string_pretty(&cli_token).unwrap();
    fs::write(&auth_path, &json).unwrap();

    // Desktop reads token
    let loaded_json = fs::read_to_string(&auth_path).unwrap();
    let desktop_token: DesktopAuthToken = serde_json::from_str(&loaded_json).unwrap();

    // Verify fields match
    assert_eq!(desktop_token.access_token, cli_token.access_token);
    assert_eq!(desktop_token.refresh_token, cli_token.refresh_token);
    assert_eq!(desktop_token.user.id, cli_token.user.id);
    assert_eq!(desktop_token.user.github_id, cli_token.user.github_id);
    assert_eq!(
        desktop_token.user.github_username,
        cli_token.user.github_username
    );
    assert_eq!(desktop_token.user.email, cli_token.user.email);
    assert_eq!(desktop_token.user.avatar_url, cli_token.user.avatar_url);

    // Verify expires_at is parseable
    let parsed_expires: DateTime<Utc> = desktop_token.expires_at.parse().unwrap();
    assert!(parsed_expires > Utc::now());
}

#[test]
fn test_desktop_token_can_be_read_by_cli() {
    // Desktop writes a token, CLI should be able to read it
    let (_temp, auth_path) = create_temp_auth_dir();

    // Desktop writes token
    let desktop_token = create_desktop_test_token();
    let json = serde_json::to_string_pretty(&desktop_token).unwrap();
    fs::write(&auth_path, &json).unwrap();

    // CLI reads token
    let loaded_json = fs::read_to_string(&auth_path).unwrap();
    let cli_token: CliAuthToken = serde_json::from_str(&loaded_json).unwrap();

    // Verify fields match
    assert_eq!(cli_token.access_token, desktop_token.access_token);
    assert_eq!(cli_token.refresh_token, desktop_token.refresh_token);
    assert_eq!(cli_token.user.id, desktop_token.user.id);
    assert_eq!(cli_token.user.github_id, desktop_token.user.github_id);
    assert_eq!(
        cli_token.user.github_username,
        desktop_token.user.github_username
    );
    assert_eq!(cli_token.user.email, desktop_token.user.email);
    assert_eq!(cli_token.user.avatar_url, desktop_token.user.avatar_url);

    // Verify expires_at was parsed correctly
    assert!(cli_token.expires_at > Utc::now());
}

#[test]
fn test_token_roundtrip_cli_to_desktop_to_cli() {
    // Verify token can survive multiple write/read cycles between CLI and Desktop
    let (_temp, auth_path) = create_temp_auth_dir();

    // Step 1: CLI writes token
    let original_cli_token = create_cli_test_token();
    let json = serde_json::to_string_pretty(&original_cli_token).unwrap();
    fs::write(&auth_path, &json).unwrap();

    // Step 2: Desktop reads and modifies token
    let loaded_json = fs::read_to_string(&auth_path).unwrap();
    let mut desktop_token: DesktopAuthToken = serde_json::from_str(&loaded_json).unwrap();
    desktop_token.access_token = "modified_by_desktop".to_string();

    // Step 3: Desktop writes token back
    let json = serde_json::to_string_pretty(&desktop_token).unwrap();
    fs::write(&auth_path, &json).unwrap();

    // Step 4: CLI reads token again
    let loaded_json = fs::read_to_string(&auth_path).unwrap();
    let final_cli_token: CliAuthToken = serde_json::from_str(&loaded_json).unwrap();

    // Verify modification persisted
    assert_eq!(final_cli_token.access_token, "modified_by_desktop");
    // But other fields unchanged
    assert_eq!(final_cli_token.user.id, original_cli_token.user.id);
}

// =============================================================================
// Token Expiry Tests
// =============================================================================

#[test]
fn test_cli_expired_token() {
    // Token that expired 1 hour ago
    let expired_token = CliAuthToken {
        access_token: "expired_token".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: Utc::now() - Duration::hours(1),
        user: CliAuthUser {
            id: "user-123".to_string(),
            github_id: 123,
            github_username: "expired".to_string(),
            email: None,
            avatar_url: None,
        },
    };

    // CLI expiry check (with 5-minute buffer)
    let is_valid = expired_token.expires_at > Utc::now() + Duration::minutes(5);
    assert!(!is_valid, "Expired token should be invalid");
}

#[test]
fn test_cli_nearly_expired_token() {
    // Token that expires in 3 minutes (within 5-minute buffer)
    let near_expiry_token = CliAuthToken {
        access_token: "near_expiry_token".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: Utc::now() + Duration::minutes(3),
        user: CliAuthUser {
            id: "user-123".to_string(),
            github_id: 123,
            github_username: "nearexpiry".to_string(),
            email: None,
            avatar_url: None,
        },
    };

    // CLI uses 5-minute buffer, so 3 minutes should be considered "needs refresh"
    let needs_refresh = near_expiry_token.expires_at <= Utc::now() + Duration::minutes(5);
    assert!(
        needs_refresh,
        "Token expiring in 3 minutes should need refresh"
    );
}

#[test]
fn test_desktop_expiry_validation() {
    // Desktop uses ISO string for expires_at
    let expired_str = (Utc::now() - Duration::hours(1)).to_rfc3339();
    let valid_str = (Utc::now() + Duration::hours(1)).to_rfc3339();

    // Parse and validate expired
    let expired_dt: DateTime<Utc> = expired_str.parse().unwrap();
    assert!(
        expired_dt <= Utc::now(),
        "Expired time should be in the past"
    );

    // Parse and validate valid
    let valid_dt: DateTime<Utc> = valid_str.parse().unwrap();
    assert!(valid_dt > Utc::now(), "Valid time should be in the future");
}

// =============================================================================
// File Permission Tests (Unix only)
// =============================================================================

#[test]
#[cfg(unix)]
fn test_auth_file_permissions() {
    let (_temp, auth_path) = create_temp_auth_dir();

    // Write token
    let token = create_cli_test_token();
    let json = serde_json::to_string_pretty(&token).unwrap();
    fs::write(&auth_path, &json).unwrap();

    // Set permissions to 0600 (as CLI and Desktop should do)
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(&auth_path, perms).unwrap();

    // Verify permissions
    let metadata = fs::metadata(&auth_path).unwrap();
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "Auth file should have 0600 permissions");
}

#[test]
#[cfg(unix)]
fn test_auth_directory_permissions() {
    let temp = TempDir::new().expect("Failed to create temp directory");
    let cas_dir = temp.path().join(".cas");

    // Create directory
    fs::create_dir_all(&cas_dir).unwrap();

    // Set permissions to 0700 (as CLI and Desktop should do)
    let perms = fs::Permissions::from_mode(0o700);
    fs::set_permissions(&cas_dir, perms).unwrap();

    // Verify permissions
    let metadata = fs::metadata(&cas_dir).unwrap();
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "Auth directory should have 0700 permissions");
}

// =============================================================================
// Optional Fields Tests
// =============================================================================

#[test]
fn test_token_without_optional_fields() {
    // Token without email and avatar_url
    let minimal_token = CliAuthToken {
        access_token: "minimal_token".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: Utc::now() + Duration::hours(1),
        user: CliAuthUser {
            id: "user-123".to_string(),
            github_id: 123,
            github_username: "minimal".to_string(),
            email: None,
            avatar_url: None,
        },
    };

    // Serialize (should skip None fields)
    let json = serde_json::to_string(&minimal_token).unwrap();
    assert!(
        !json.contains("\"email\""),
        "None email should be skipped in serialization"
    );
    assert!(
        !json.contains("\"avatar_url\""),
        "None avatar_url should be skipped in serialization"
    );

    // Desktop should still be able to read it
    let desktop_token: DesktopAuthToken = serde_json::from_str(&json).unwrap();
    assert!(desktop_token.user.email.is_none());
    assert!(desktop_token.user.avatar_url.is_none());
}

#[test]
fn test_desktop_token_with_null_fields() {
    // Desktop might write explicit nulls for optional fields
    let json = r#"{
        "access_token": "test",
        "refresh_token": "refresh",
        "expires_at": "2030-01-01T00:00:00Z",
        "user": {
            "id": "user-123",
            "github_id": 123,
            "github_username": "testuser",
            "email": null,
            "avatar_url": null
        }
    }"#;

    // CLI should handle explicit nulls
    let cli_token: CliAuthToken = serde_json::from_str(json).unwrap();
    assert!(cli_token.user.email.is_none());
    assert!(cli_token.user.avatar_url.is_none());
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn test_load_nonexistent_token() {
    let (_temp, auth_path) = create_temp_auth_dir();

    // File doesn't exist
    assert!(!auth_path.exists());

    // Attempting to read should return an error or empty
    let result = fs::read_to_string(&auth_path);
    assert!(result.is_err(), "Reading nonexistent file should error");
}

#[test]
fn test_load_invalid_json() {
    let (_temp, auth_path) = create_temp_auth_dir();

    // Write invalid JSON
    fs::write(&auth_path, "not valid json {{{").unwrap();

    // Attempting to parse should fail
    let content = fs::read_to_string(&auth_path).unwrap();
    let result: Result<CliAuthToken, _> = serde_json::from_str(&content);
    assert!(result.is_err(), "Parsing invalid JSON should error");
}

#[test]
fn test_load_incomplete_token() {
    let (_temp, auth_path) = create_temp_auth_dir();

    // Write JSON missing required fields
    let incomplete_json = r#"{"access_token": "test"}"#;
    fs::write(&auth_path, incomplete_json).unwrap();

    // Attempting to parse should fail
    let content = fs::read_to_string(&auth_path).unwrap();
    let result: Result<CliAuthToken, _> = serde_json::from_str(&content);
    assert!(result.is_err(), "Parsing incomplete token should error");
}

// =============================================================================
// Date Format Compatibility Tests
// =============================================================================

#[test]
fn test_various_date_formats() {
    // Test that various RFC3339/ISO8601 formats work
    let test_dates = vec![
        "2030-01-01T00:00:00Z",
        "2030-01-01T00:00:00+00:00",
        "2030-01-01T12:30:45.123Z",
        "2030-01-01T00:00:00.000000Z",
    ];

    for date_str in test_dates {
        let json = format!(
            r#"{{
                "access_token": "test",
                "refresh_token": "refresh",
                "expires_at": "{date_str}",
                "user": {{
                    "id": "user-123",
                    "github_id": 123,
                    "github_username": "testuser",
                    "email": null,
                    "avatar_url": null
                }}
            }}"#
        );

        let result: Result<CliAuthToken, _> = serde_json::from_str(&json);
        assert!(
            result.is_ok(),
            "Should parse date format: {} - error: {:?}",
            date_str,
            result.err()
        );
    }
}
