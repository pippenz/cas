//! Error types for MCP server
//!
//! This module provides MCP-specific error types that wrap CoreError
//! and add MCP protocol-specific error variants.

use thiserror::Error;

/// MCP server error type
#[derive(Error, Debug)]
pub enum McpError {
    /// Error from core business logic
    #[error("core error: {0}")]
    Core(#[from] cas_core::CoreError),

    /// Invalid tool parameters
    #[error("invalid parameters: {0}")]
    InvalidParameters(String),

    /// Tool not found
    #[error("tool not found: {0}")]
    ToolNotFound(String),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Transport error
    #[error("transport error: {0}")]
    Transport(String),

    /// I/O error
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Generic error
    #[error("{0}")]
    Other(String),
}

/// Result type alias using McpError
pub type Result<T> = std::result::Result<T, McpError>;

#[cfg(test)]
mod tests {
    use crate::error::*;

    #[test]
    fn test_invalid_parameters_error() {
        let err = McpError::InvalidParameters("missing required field".to_string());
        assert!(err.to_string().contains("invalid parameters"));
    }

    #[test]
    fn test_tool_not_found_error() {
        let err = McpError::ToolNotFound("unknown_tool".to_string());
        assert!(err.to_string().contains("tool not found"));
        assert!(err.to_string().contains("unknown_tool"));
    }

    #[test]
    fn test_transport_error() {
        let err = McpError::Transport("connection lost".to_string());
        assert!(err.to_string().contains("transport error"));
    }

    #[test]
    fn test_other_error() {
        let err = McpError::Other("something went wrong".to_string());
        assert_eq!(err.to_string(), "something went wrong");
    }
}
