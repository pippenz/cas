//! Mock cloud server for testing cloud sync operations
//!
//! Provides a wiremock-based mock server that simulates the CAS Cloud API.
#![allow(dead_code)]

use serde_json::json;
use wiremock::matchers::{header, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Mock cloud server for testing sync operations
pub struct CloudMockServer {
    /// The underlying wiremock server
    pub server: MockServer,
    /// The endpoint URL
    pub endpoint: String,
}

impl CloudMockServer {
    /// Start a new mock cloud server
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        let endpoint = server.uri();
        Self { server, endpoint }
    }

    /// Mock successful push response
    pub async fn mock_push_success(
        &self,
        entries: usize,
        tasks: usize,
        rules: usize,
        skills: usize,
    ) {
        Mock::given(method("POST"))
            .and(path("/api/sync/push"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": { "inserted": entries, "updated": 0 },
                "tasks": { "inserted": tasks, "updated": 0 },
                "rules": { "inserted": rules, "updated": 0 },
                "skills": { "inserted": skills, "updated": 0 }
            })))
            .mount(&self.server)
            .await;
    }

    /// Mock successful pull response with data
    pub async fn mock_pull_with_data(
        &self,
        entries: Vec<serde_json::Value>,
        tasks: Vec<serde_json::Value>,
    ) {
        Mock::given(method("GET"))
            .and(path("/api/sync/pull"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": entries,
                "tasks": tasks,
                "rules": [],
                "skills": [],
                "pulled_at": chrono::Utc::now().to_rfc3339()
            })))
            .mount(&self.server)
            .await;
    }

    /// Mock conflict scenario - returns entry that conflicts with local
    pub async fn mock_pull_with_conflict(&self, entry_id: &str, remote_content: &str) {
        let now = chrono::Utc::now();
        Mock::given(method("GET"))
            .and(path("/api/sync/pull"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{
                    "id": entry_id,
                    "content": remote_content,
                    "entry_type": "learning",
                    "created": now.to_rfc3339(),
                    "last_accessed": now.to_rfc3339(),
                    "helpful_count": 0,
                    "harmful_count": 0,
                    "archived": false
                }],
                "tasks": [],
                "rules": [],
                "skills": [],
                "pulled_at": now.to_rfc3339()
            })))
            .mount(&self.server)
            .await;
    }

    /// Mock rate limit response (429)
    pub async fn mock_rate_limit(&self) {
        Mock::given(method("POST"))
            .and(path_regex("/api/.*"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_json(json!({
                        "error": "rate_limited",
                        "message": "Too many requests",
                        "retry_after": 60
                    }))
                    .insert_header("Retry-After", "60"),
            )
            .mount(&self.server)
            .await;
    }

    /// Mock server error (500)
    pub async fn mock_server_error(&self) {
        Mock::given(method("POST"))
            .and(path_regex("/api/.*"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "error": "internal_error",
                "message": "Internal server error"
            })))
            .mount(&self.server)
            .await;
    }
}

/// Create a sample entry JSON for testing
pub fn sample_entry_json(id: &str, content: &str) -> serde_json::Value {
    let now = chrono::Utc::now();
    json!({
        "id": id,
        "content": content,
        "entry_type": "learning",
        "scope": "project",
        "created": now.to_rfc3339(),
        "last_accessed": now.to_rfc3339(),
        "helpful_count": 0,
        "harmful_count": 0,
        "archived": false,
        "tags": [],
        "memory_tier": "working"
    })
}

/// Create a sample task JSON for testing
pub fn sample_task_json(id: &str, title: &str) -> serde_json::Value {
    let now = chrono::Utc::now();
    json!({
        "id": id,
        "title": title,
        "status": "open",
        "priority": 2,
        "task_type": "task",
        "created_at": now.to_rfc3339(),
        "updated_at": now.to_rfc3339()
    })
}
