//! Transcript parsing for Claude Code sessions
//!
//! Parses JSONL transcript files to extract assistant messages for
//! completion detection in iteration loops.

use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::error::CoreError;

/// A content block in an assistant message
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(other)]
    Other,
}

/// The message structure within a transcript entry
#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptMessage {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    /// Model used for assistant messages (e.g., "claude-opus-4-5-20251101")
    #[serde(default)]
    pub model: Option<String>,
}

/// A transcript entry (one line of JSONL)
#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(default)]
    pub message: Option<TranscriptMessage>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    /// Claude Code version (e.g., "2.1.14")
    #[serde(default)]
    pub version: Option<String>,
}

/// Session metadata extracted from transcript
#[derive(Debug, Clone, Default)]
pub struct TranscriptMetadata {
    /// Model used for generating responses (e.g., "claude-opus-4-5-20251101")
    pub model: Option<String>,
    /// Claude Code version (e.g., "2.1.14")
    pub tool_version: Option<String>,
}

/// Extract text content from the last assistant message in a transcript
///
/// Reads the transcript JSONL file and finds the most recent assistant
/// message, returning all text content blocks concatenated.
pub fn get_last_assistant_text(transcript_path: &Path) -> Result<Option<String>, CoreError> {
    if !transcript_path.exists() {
        return Ok(None);
    }

    let file = File::open(transcript_path).map_err(|e| {
        CoreError::Io(std::io::Error::new(
            e.kind(),
            format!("Failed to open transcript: {e}"),
        ))
    })?;

    let reader = BufReader::new(file);
    let mut last_assistant_text: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        // Parse the JSONL entry
        let entry: TranscriptEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Only process assistant messages
        if entry.entry_type != "assistant" {
            continue;
        }

        // Extract text from content blocks
        if let Some(ref message) = entry.message {
            let mut text_parts: Vec<String> = Vec::new();

            for block in &message.content {
                if let ContentBlock::Text { text } = block {
                    text_parts.push(text.clone());
                }
            }

            if !text_parts.is_empty() {
                last_assistant_text = Some(text_parts.join("\n"));
            }
        }
    }

    Ok(last_assistant_text)
}

/// Check if a completion promise is present in the last assistant message
///
/// Looks for the pattern `<promise>TEXT</promise>` in the transcript.
pub fn check_promise_in_transcript(
    transcript_path: &Path,
    promise: &str,
) -> Result<bool, CoreError> {
    let text = get_last_assistant_text(transcript_path)?;

    match text {
        Some(content) => {
            let pattern = format!("<promise>{promise}</promise>");
            Ok(content.contains(&pattern))
        }
        None => Ok(false),
    }
}

/// Get the last N assistant messages from a transcript
///
/// Useful for getting context about recent work in the loop.
pub fn get_recent_assistant_messages(
    transcript_path: &Path,
    count: usize,
) -> Result<Vec<String>, CoreError> {
    if !transcript_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(transcript_path).map_err(|e| {
        CoreError::Io(std::io::Error::new(
            e.kind(),
            format!("Failed to open transcript: {e}"),
        ))
    })?;

    let reader = BufReader::new(file);
    let mut messages: Vec<String> = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        let entry: TranscriptEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.entry_type != "assistant" {
            continue;
        }

        if let Some(ref message) = entry.message {
            let mut text_parts: Vec<String> = Vec::new();

            for block in &message.content {
                if let ContentBlock::Text { text } = block {
                    text_parts.push(text.clone());
                }
            }

            if !text_parts.is_empty() {
                messages.push(text_parts.join("\n"));
            }
        }
    }

    // Return the last N messages
    let start = messages.len().saturating_sub(count);
    Ok(messages[start..].to_vec())
}

/// Extract session metadata from a transcript (model, tool version)
///
/// Scans the transcript for model info (from assistant messages) and
/// tool version (from any entry with version field).
pub fn extract_transcript_metadata(
    transcript_path: &Path,
) -> Result<TranscriptMetadata, CoreError> {
    if !transcript_path.exists() {
        return Ok(TranscriptMetadata::default());
    }

    let file = File::open(transcript_path).map_err(|e| {
        CoreError::Io(std::io::Error::new(
            e.kind(),
            format!("Failed to open transcript: {e}"),
        ))
    })?;

    let reader = BufReader::new(file);
    let mut metadata = TranscriptMetadata::default();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        let entry: TranscriptEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Extract tool version from any entry (usually first entry)
        if metadata.tool_version.is_none() {
            if let Some(version) = entry.version {
                metadata.tool_version = Some(version);
            }
        }

        // Extract model from assistant messages
        if metadata.model.is_none() && entry.entry_type == "assistant" {
            if let Some(ref message) = entry.message {
                if let Some(ref model) = message.model {
                    metadata.model = Some(model.clone());
                }
            }
        }

        // Once we have both, we can stop scanning
        if metadata.model.is_some() && metadata.tool_version.is_some() {
            break;
        }
    }

    Ok(metadata)
}

/// Parse a transcript file into a list of Messages (for blame v2)
///
/// Converts TranscriptEntry format to cas_types::Message format for storage.
/// Excludes tool results (verbose) per spec guidance.
pub fn parse_transcript_to_messages(
    transcript_path: &Path,
) -> Result<Vec<cas_types::Message>, CoreError> {
    if !transcript_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(transcript_path).map_err(|e| {
        CoreError::Io(std::io::Error::new(
            e.kind(),
            format!("Failed to open transcript: {e}"),
        ))
    })?;

    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        let entry: TranscriptEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Parse timestamp if available
        let timestamp = entry
            .timestamp
            .as_ref()
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        match entry.entry_type.as_str() {
            "user" => {
                if let Some(ref message) = entry.message {
                    for block in &message.content {
                        if let ContentBlock::Text { text } = block {
                            messages.push(cas_types::Message {
                                role: cas_types::MessageRole::User,
                                content: text.clone(),
                                tool_name: None,
                                tool_input: None,
                                timestamp,
                            });
                        }
                    }
                }
            }
            "assistant" => {
                if let Some(ref message) = entry.message {
                    for block in &message.content {
                        match block {
                            ContentBlock::Text { text } => {
                                messages.push(cas_types::Message {
                                    role: cas_types::MessageRole::Assistant,
                                    content: text.clone(),
                                    tool_name: None,
                                    tool_input: None,
                                    timestamp,
                                });
                            }
                            ContentBlock::ToolUse { name, input, .. } => {
                                // Store tool use but not tool results (too verbose)
                                messages.push(cas_types::Message {
                                    role: cas_types::MessageRole::ToolUse,
                                    content: String::new(),
                                    tool_name: Some(name.clone()),
                                    tool_input: Some(input.clone()),
                                    timestamp,
                                });
                            }
                            ContentBlock::Other => {}
                        }
                    }
                }
            }
            // Skip tool_result entries (too verbose per spec)
            _ => {}
        }
    }

    Ok(messages)
}

#[cfg(test)]
mod tests {
    use crate::hooks::transcript::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_transcript(entries: &[&str]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        for entry in entries {
            writeln!(file, "{entry}").unwrap();
        }
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_get_last_assistant_text() {
        let entries = vec![
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Hello"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hi there!"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Final message"}]}}"#,
        ];

        let file = create_test_transcript(&entries);
        let result = get_last_assistant_text(file.path()).unwrap();

        assert_eq!(result, Some("Final message".to_string()));
    }

    #[test]
    fn test_check_promise_in_transcript() {
        let entries = vec![
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Working on it..."}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"All done! <promise>DONE</promise>"}]}}"#,
        ];

        let file = create_test_transcript(&entries);

        assert!(check_promise_in_transcript(file.path(), "DONE").unwrap());
        assert!(!check_promise_in_transcript(file.path(), "COMPLETE").unwrap());
    }

    #[test]
    fn test_get_recent_assistant_messages() {
        let entries = vec![
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Message 1"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Message 2"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Message 3"}]}}"#,
        ];

        let file = create_test_transcript(&entries);
        let messages = get_recent_assistant_messages(file.path(), 2).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], "Message 2");
        assert_eq!(messages[1], "Message 3");
    }

    #[test]
    fn test_empty_transcript() {
        let file = create_test_transcript(&[]);
        let result = get_last_assistant_text(file.path()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_no_assistant_messages() {
        let entries = vec![
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Hello"}]}}"#,
            r#"{"type":"queue-operation","operation":"enqueue"}"#,
        ];

        let file = create_test_transcript(&entries);
        let result = get_last_assistant_text(file.path()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_multiple_content_blocks() {
        let entries = vec![
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Part 1"},{"type":"text","text":"Part 2"}]}}"#,
        ];

        let file = create_test_transcript(&entries);
        let result = get_last_assistant_text(file.path()).unwrap();

        assert_eq!(result, Some("Part 1\nPart 2".to_string()));
    }

    #[test]
    fn test_tool_use_blocks_ignored() {
        let entries = vec![
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Using tool"},{"type":"tool_use","id":"123","name":"Read","input":{}}]}}"#,
        ];

        let file = create_test_transcript(&entries);
        let result = get_last_assistant_text(file.path()).unwrap();

        assert_eq!(result, Some("Using tool".to_string()));
    }

    #[test]
    fn test_nonexistent_file() {
        let path = Path::new("/nonexistent/transcript.jsonl");
        let result = get_last_assistant_text(path).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_transcript_to_messages() {
        let entries = vec![
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Hello, please help me"}]},"timestamp":"2026-01-24T10:00:00Z"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I'll help you with that"}]},"timestamp":"2026-01-24T10:00:01Z"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file":"test.rs"}}]},"timestamp":"2026-01-24T10:00:02Z"}"#,
        ];

        let file = create_test_transcript(&entries);
        let messages = parse_transcript_to_messages(file.path()).unwrap();

        assert_eq!(messages.len(), 3);

        // Check user message
        assert_eq!(messages[0].role, cas_types::MessageRole::User);
        assert_eq!(messages[0].content, "Hello, please help me");

        // Check assistant message
        assert_eq!(messages[1].role, cas_types::MessageRole::Assistant);
        assert_eq!(messages[1].content, "I'll help you with that");

        // Check tool use message
        assert_eq!(messages[2].role, cas_types::MessageRole::ToolUse);
        assert_eq!(messages[2].tool_name, Some("Read".to_string()));
    }

    #[test]
    fn test_parse_transcript_empty_file() {
        let file = create_test_transcript(&[]);
        let messages = parse_transcript_to_messages(file.path()).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_transcript_nonexistent_file() {
        let path = Path::new("/nonexistent/transcript.jsonl");
        let messages = parse_transcript_to_messages(path).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_extract_transcript_metadata() {
        // Real transcript format with model and version
        let entries = vec![
            r#"{"type":"user","version":"2.1.14","message":{"role":"user","content":[{"type":"text","text":"Hello"}]}}"#,
            r#"{"type":"assistant","version":"2.1.14","message":{"role":"assistant","model":"claude-opus-4-5-20251101","content":[{"type":"text","text":"Hi there!"}]}}"#,
        ];

        let file = create_test_transcript(&entries);
        let metadata = extract_transcript_metadata(file.path()).unwrap();

        assert_eq!(metadata.model, Some("claude-opus-4-5-20251101".to_string()));
        assert_eq!(metadata.tool_version, Some("2.1.14".to_string()));
    }

    #[test]
    fn test_extract_transcript_metadata_no_model() {
        // Transcript without model info (e.g., older format)
        let entries = vec![
            r#"{"type":"user","version":"2.0.50","message":{"role":"user","content":[{"type":"text","text":"Hello"}]}}"#,
            r#"{"type":"assistant","version":"2.0.50","message":{"role":"assistant","content":[{"type":"text","text":"Hi there!"}]}}"#,
        ];

        let file = create_test_transcript(&entries);
        let metadata = extract_transcript_metadata(file.path()).unwrap();

        assert_eq!(metadata.model, None);
        assert_eq!(metadata.tool_version, Some("2.0.50".to_string()));
    }

    #[test]
    fn test_extract_transcript_metadata_nonexistent_file() {
        let path = Path::new("/nonexistent/transcript.jsonl");
        let metadata = extract_transcript_metadata(path).unwrap();

        assert_eq!(metadata.model, None);
        assert_eq!(metadata.tool_version, None);
    }

    #[test]
    fn test_extract_transcript_metadata_empty() {
        let file = create_test_transcript(&[]);
        let metadata = extract_transcript_metadata(file.path()).unwrap();

        assert_eq!(metadata.model, None);
        assert_eq!(metadata.tool_version, None);
    }
}
