//! Unix socket for hook-daemon communication
//!
//! Provides instant event delivery from hooks to the daemon, replacing file polling.
//!
//! # Protocol
//!
//! Events are sent as newline-delimited JSON over Unix socket.
//! The daemon listens at `.cas/daemon.sock`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};

/// Events sent from hooks to the daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonEvent {
    /// Session started - register agent and PID mapping
    /// agent_name comes from CAS_AGENT_NAME in the hook's environment (Claude Code process)
    /// agent_role comes from CAS_AGENT_ROLE in the hook's environment (set by factory mode)
    SessionStart {
        session_id: String,
        agent_name: Option<String>,
        /// Agent role from CAS_AGENT_ROLE (e.g., "worker", "supervisor")
        #[serde(default)]
        agent_role: Option<String>,
        /// Claude Code's PID (the parent of the hook process)
        cc_pid: u32,
        /// Worker's clone path from CAS_CLONE_PATH (for factory mode workers)
        #[serde(default)]
        clone_path: Option<String>,
    },
    /// Session ended - clear agent cache and PID mapping
    SessionEnd {
        session_id: String,
        /// Claude Code's PID to remove from mapping
        cc_pid: Option<u32>,
    },
    /// Query session ID for a given Claude Code PID
    GetSession { cc_pid: u32 },
    /// Ping - check if daemon is alive
    Ping,
    /// Worker activity for supervisor visibility
    WorkerActivity {
        /// Session ID of the worker
        session_id: String,
        /// Event type string (maps to EventType variant)
        event_type: String,
        /// Human-readable description
        description: String,
        /// Optional entity ID (task ID, file path, etc.)
        #[serde(default)]
        entity_id: Option<String>,
    },
}

/// Response from daemon to hooks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonResponse {
    /// Acknowledgment
    Ok,
    /// Pong response to ping
    Pong,
    /// Session ID response to GetSession query
    Session { session_id: String },
    /// No session found for the given PID
    NoSession,
    /// Error
    Error { message: String },
}

/// Get the socket path for a CAS root
pub fn socket_path(cas_root: &Path) -> PathBuf {
    cas_root.join("daemon.sock")
}

/// Create and bind the Unix socket listener
///
/// Checks if an existing socket is live before removing it.
/// Returns an error if another daemon is already listening.
pub fn create_listener(cas_root: &Path) -> std::io::Result<UnixListener> {
    use std::os::unix::net::UnixStream as StdUnixStream;

    let path = socket_path(cas_root);

    // Check if socket exists and has an active listener
    if path.exists() {
        // Try to connect - if successful, another daemon is listening
        match StdUnixStream::connect(&path) {
            Ok(_) => {
                // Another daemon is already listening - don't steal the socket
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "Another daemon is already listening on this socket",
                ));
            }
            Err(e) => {
                // Connection failed - socket is stale
                // Only remove if it's a connection refused or not found error
                if e.kind() == std::io::ErrorKind::ConnectionRefused
                    || e.kind() == std::io::ErrorKind::NotFound
                {
                    std::fs::remove_file(&path)?;
                } else {
                    // Some other error - try to remove anyway but don't fail hard
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    UnixListener::bind(&path)
}

/// Cleanup the socket file on shutdown
pub fn cleanup_socket(cas_root: &Path) {
    let path = socket_path(cas_root);
    let _ = std::fs::remove_file(path);
}

/// Send an event to the daemon (called by hooks)
///
/// This is a synchronous blocking call suitable for hook context.
pub fn send_event(cas_root: &Path, event: &DaemonEvent) -> std::io::Result<DaemonResponse> {
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixStream as StdUnixStream;

    let path = socket_path(cas_root);
    let mut stream = StdUnixStream::connect(&path)?;

    // Set timeout for hook context (don't block too long)
    stream.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_millis(500)))?;

    // Send event as JSON line
    let json = serde_json::to_string(event)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    writeln!(stream, "{json}")?;
    stream.flush()?;

    // Read response
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    serde_json::from_str(&line)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// Read event from socket connection (async, called by daemon)
pub async fn read_event(stream: &mut UnixStream) -> Option<DaemonEvent> {
    use tokio::io::AsyncBufReadExt;

    let mut reader = BufReader::new(&mut *stream);
    let mut line = String::new();

    match reader.read_line(&mut line).await {
        Ok(0) => None, // EOF
        Ok(_) => match serde_json::from_str::<DaemonEvent>(&line) {
            Ok(event) => Some(event),
            Err(e) => {
                eprintln!("[CAS] Invalid event from hook: {e}");
                None
            }
        },
        Err(e) => {
            eprintln!("[CAS] Error reading from hook socket: {e}");
            None
        }
    }
}

/// Send response back to hook (async)
pub async fn send_response(
    stream: &mut UnixStream,
    response: &DaemonResponse,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let json = serde_json::to_string(response)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    stream.write_all(json.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await
}
