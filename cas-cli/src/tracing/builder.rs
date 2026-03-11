use crate::tracing::{TraceEvent, TraceEventType};

/// Helper to generate unique trace IDs
pub fn generate_trace_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();

    format!("tr-{timestamp:x}")
}

/// Builder for constructing TraceEvent instances
///
/// Provides a fluent API for building trace events with proper defaults.
pub struct TraceBuilder {
    event_type: TraceEventType,
    session_id: Option<String>,
    input: String,
    start_time: std::time::Instant,
    duration_override: Option<u64>,
    metadata: String,
    output: String,
}

impl TraceBuilder {
    /// Create a new trace builder for the given event type
    pub fn new(event_type: TraceEventType) -> Self {
        Self {
            event_type,
            session_id: None,
            input: "{}".to_string(),
            start_time: std::time::Instant::now(),
            duration_override: None,
            metadata: "{}".to_string(),
            output: "{}".to_string(),
        }
    }

    /// Set the session ID
    pub fn session(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    /// Set the input data
    pub fn input(mut self, input: &serde_json::Value) -> Self {
        self.input = serde_json::to_string(input).unwrap_or_default();
        self
    }

    /// Set metadata
    pub fn metadata(mut self, metadata: &serde_json::Value) -> Self {
        self.metadata = serde_json::to_string(metadata).unwrap_or_default();
        self
    }

    /// Set output data
    pub fn output(mut self, output: &serde_json::Value) -> Self {
        self.output = serde_json::to_string(output).unwrap_or_default();
        self
    }

    /// Set explicit duration in milliseconds (overrides automatic timing)
    pub fn duration_ms(mut self, duration: u64) -> Self {
        self.duration_override = Some(duration);
        self
    }

    /// Get the effective duration (override or elapsed)
    fn get_duration(&self) -> u64 {
        self.duration_override
            .unwrap_or_else(|| self.start_time.elapsed().as_millis() as u64)
    }

    /// Finish building with a successful result
    pub fn finish_success(self, output: &serde_json::Value) -> TraceEvent {
        let duration = self.get_duration();
        TraceEvent {
            id: generate_trace_id(),
            event_type: self.event_type,
            timestamp: chrono::Utc::now(),
            session_id: self.session_id,
            duration_ms: duration,
            input: self.input,
            output: serde_json::to_string(output).unwrap_or_default(),
            metadata: self.metadata,
            success: true,
            error: None,
        }
    }

    /// Finish building with an error
    pub fn finish_error(self, error: &str) -> TraceEvent {
        let duration = self.get_duration();
        TraceEvent {
            id: generate_trace_id(),
            event_type: self.event_type,
            timestamp: chrono::Utc::now(),
            session_id: self.session_id,
            duration_ms: duration,
            input: self.input,
            output: self.output,
            metadata: self.metadata,
            success: false,
            error: Some(error.to_string()),
        }
    }

    /// Finish building with custom success flag
    pub fn finish(self, success: bool, error: Option<&str>) -> TraceEvent {
        let duration = self.get_duration();
        TraceEvent {
            id: generate_trace_id(),
            event_type: self.event_type,
            timestamp: chrono::Utc::now(),
            session_id: self.session_id,
            duration_ms: duration,
            input: self.input,
            output: self.output,
            metadata: self.metadata,
            success,
            error: error.map(|s| s.to_string()),
        }
    }
}

/// Trace timer for measuring duration
pub struct TraceTimer {
    start: std::time::Instant,
}

impl TraceTimer {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

impl Default for TraceTimer {
    fn default() -> Self {
        Self::new()
    }
}
