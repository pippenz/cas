//! Stub tracing module for cas-store
//!
//! Provides no-op tracing to avoid dependency on cas-cli's tracing module.

/// Timer that does nothing
pub struct TraceTimer;

impl TraceTimer {
    pub fn new() -> Self {
        Self
    }

    pub fn elapsed_ms(&self) -> u64 {
        0
    }
}

impl Default for TraceTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Dev tracer that does nothing
pub struct DevTracer;

impl DevTracer {
    pub fn get() -> Option<Self> {
        None
    }

    pub fn should_trace_store_ops(&self) -> bool {
        false
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::result_unit_err)]
    pub fn record_store_op(
        &self,
        _op: &str,
        _store_type: &str,
        _ids: &[&str],
        _count: usize,
        _elapsed_ms: u64,
        _success: bool,
        _error: Option<&str>,
    ) -> Result<(), ()> {
        Ok(())
    }
}
