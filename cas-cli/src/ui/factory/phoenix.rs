//! Shared Phoenix channel wire protocol utilities
//!
//! Phoenix channels use a JSON array wire format:
//! `[join_ref, ref, topic, event, payload]`
//!
//! WebSocket URL: `wss://{endpoint}/socket/websocket?token={token}&vsn=2.0.0`

use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

/// Ref counter for Phoenix protocol messages
static MSG_REF: AtomicU64 = AtomicU64::new(1);

/// Get the next unique message ref (monotonically incrementing)
pub fn next_ref() -> String {
    MSG_REF.fetch_add(1, Ordering::Relaxed).to_string()
}

/// Encode a Phoenix channel message as JSON
///
/// Format: `[join_ref, ref, topic, event, payload]`
pub fn encode_msg(join_ref: Option<&str>, topic: &str, event: &str, payload: &Value) -> String {
    serde_json::json!([join_ref, next_ref(), topic, event, payload]).to_string()
}

/// Build a Phoenix WebSocket URL from a cloud HTTP endpoint
///
/// Converts `https://` → `wss://` and `http://` → `ws://`, appends socket path.
pub fn ws_url(endpoint: &str, token: &str) -> String {
    let base = endpoint
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    format!("{base}/socket/websocket?token={token}&vsn=2.0.0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_url_https() {
        let url = ws_url("https://cas.dev", "test_token");
        assert_eq!(
            url,
            "wss://cas.dev/socket/websocket?token=test_token&vsn=2.0.0"
        );
    }

    #[test]
    fn test_ws_url_http() {
        let url = ws_url("http://localhost:4000", "tok123");
        assert_eq!(
            url,
            "ws://localhost:4000/socket/websocket?token=tok123&vsn=2.0.0"
        );
    }

    #[test]
    fn test_encode_msg_join() {
        let payload = serde_json::json!({"role": "daemon"});
        let msg = encode_msg(Some("1"), "factory:abc", "phx_join", &payload);
        let parsed: Vec<Value> = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed[0], "1");
        assert!(parsed[1].is_string());
        assert_eq!(parsed[2], "factory:abc");
        assert_eq!(parsed[3], "phx_join");
        assert_eq!(parsed[4]["role"], "daemon");
    }

    #[test]
    fn test_encode_msg_no_join_ref() {
        let payload = serde_json::json!({"key": "value"});
        let msg = encode_msg(None, "factory:abc", "factory.state", &payload);
        let parsed: Vec<Value> = serde_json::from_str(&msg).unwrap();
        assert!(parsed[0].is_null());
        assert_eq!(parsed[3], "factory.state");
    }

    #[test]
    fn test_refs_increment() {
        let r1 = next_ref();
        let r2 = next_ref();
        let n1: u64 = r1.parse().unwrap();
        let n2: u64 = r2.parse().unwrap();
        assert_eq!(n2, n1 + 1);
    }
}
