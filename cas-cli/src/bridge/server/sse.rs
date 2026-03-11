use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cas_store::{EventStore, SqliteEventStore};
use tiny_http::StatusCode;

use crate::bridge::server::http::{
    error_response, parse_bool_query, parse_string_query, parse_u64_query, write_chunked,
    write_chunked_end,
};
use crate::bridge::server::session::{
    allowed_agent_names, build_status_json, cas_root_for_session_with_fallback,
    filter_events_for_session_agents, resolve_session_by_name,
};
use crate::bridge::server::types::{ActivityJson, InboxPollJson, session_json};
use crate::store::open_supervisor_queue_store;

#[derive(Debug)]
struct SseSessionStream {
    cas_root: std::path::PathBuf,
    session: crate::ui::factory::SessionInfo,
    inbox_id: String,
    shutdown: Arc<AtomicBool>,
    poll_ms: u64,
    heartbeat_ms: u64,
    activity_limit: usize,
    inbox_limit: usize,
    status_interval_ms: u64,
    last_activity_id: i64,
    last_status_at: std::time::Instant,
    last_emit_at: std::time::Instant,
    buf: std::io::Cursor<Vec<u8>>,
    done: bool,
}

impl SseSessionStream {
    #[allow(clippy::too_many_arguments)]
    fn new(
        cas_root: std::path::PathBuf,
        session: crate::ui::factory::SessionInfo,
        inbox_id: String,
        shutdown: Arc<AtomicBool>,
        poll_ms: u64,
        heartbeat_ms: u64,
        activity_limit: usize,
        inbox_limit: usize,
        status_interval_ms: u64,
        last_activity_id: i64,
    ) -> Self {
        // Ensure the response can start immediately (some clients/proxies won't
        // treat the connection as established until at least one byte is sent).
        let initial = b": connected\n\n".to_vec();
        Self {
            cas_root,
            session,
            inbox_id,
            shutdown,
            poll_ms,
            heartbeat_ms,
            activity_limit,
            inbox_limit,
            status_interval_ms,
            last_activity_id,
            last_status_at: std::time::Instant::now(),
            last_emit_at: std::time::Instant::now(),
            buf: std::io::Cursor::new(initial),
            done: false,
        }
    }

    fn push_sse_event(out: &mut Vec<u8>, event: &str, data: &str) {
        out.extend_from_slice(b"event: ");
        out.extend_from_slice(event.as_bytes());
        out.extend_from_slice(b"\n");
        if data.is_empty() {
            out.extend_from_slice(b"data:\n\n");
            return;
        }
        for line in data.lines() {
            out.extend_from_slice(b"data: ");
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\n");
        }
        out.extend_from_slice(b"\n");
    }

    fn push_heartbeat(out: &mut Vec<u8>) {
        // Comment frame to keep the connection alive.
        out.extend_from_slice(b": heartbeat\n\n");
    }

    fn fill_next(&mut self) -> std::io::Result<()> {
        if self.done {
            self.buf = std::io::Cursor::new(Vec::new());
            return Ok(());
        }

        let allowed = allowed_agent_names(&self.session);

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                self.done = true;
                self.buf = std::io::Cursor::new(Vec::new());
                return Ok(());
            }

            let mut out: Vec<u8> = Vec::new();

            // Status snapshot at a lower frequency (optional, if interval > 0).
            if self.status_interval_ms > 0
                && self.last_status_at.elapsed()
                    >= std::time::Duration::from_millis(self.status_interval_ms)
            {
                match build_status_json(&self.session, &self.cas_root, self.activity_limit) {
                    Ok(status) => {
                        let s = serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string());
                        Self::push_sse_event(&mut out, "status", &s);
                        self.last_status_at = std::time::Instant::now();
                    }
                    Err(e) => {
                        let s = serde_json::to_string(&serde_json::json!({
                            "schema_version": 1,
                            "error": { "code": "status_error", "message": e.to_string() }
                        }))
                        .unwrap_or_else(|_| "{}".to_string());
                        Self::push_sse_event(&mut out, "error", &s);
                        self.done = true;
                        self.buf = std::io::Cursor::new(out);
                        self.last_emit_at = std::time::Instant::now();
                        return Ok(());
                    }
                }
            }

            // Activity: list recent and filter by allowed agents.
            match SqliteEventStore::open(&self.cas_root).and_then(|store| store.list_recent(200)) {
                Ok(mut activity) => {
                    filter_events_for_session_agents(&mut activity, &allowed);
                    if self.last_activity_id > 0 {
                        activity.retain(|e| e.id > self.last_activity_id);
                    }
                    let activity_limit = self.activity_limit.clamp(1, 200);
                    if activity.len() > activity_limit {
                        activity.truncate(activity_limit);
                    }
                    if !activity.is_empty() {
                        let latest_id = activity
                            .iter()
                            .map(|e| e.id)
                            .max()
                            .unwrap_or(self.last_activity_id);
                        self.last_activity_id = latest_id;
                        let s = serde_json::to_string(&ActivityJson {
                            schema_version: 1,
                            session: session_json(&self.session),
                            activity,
                            latest_id: Some(latest_id),
                        })
                        .unwrap_or_else(|_| "{}".to_string());
                        Self::push_sse_event(&mut out, "activity", &s);
                    }
                }
                Err(e) => {
                    let s = serde_json::to_string(&serde_json::json!({
                        "schema_version": 1,
                        "error": { "code": "activity_error", "message": e.to_string() }
                    }))
                    .unwrap_or_else(|_| "{}".to_string());
                    Self::push_sse_event(&mut out, "error", &s);
                    self.done = true;
                    self.buf = std::io::Cursor::new(out);
                    self.last_emit_at = std::time::Instant::now();
                    return Ok(());
                }
            }

            // Inbox: poll external inbox (marks processed).
            match open_supervisor_queue_store(&self.cas_root) {
                Ok(q) => match q.poll(&self.inbox_id, self.inbox_limit) {
                    Ok(notifications) => {
                        if !notifications.is_empty() {
                            let s = serde_json::to_string(&InboxPollJson {
                                schema_version: 1,
                                session: session_json(&self.session),
                                inbox_id: self.inbox_id.clone(),
                                polled: notifications.len(),
                                notifications,
                            })
                            .unwrap_or_else(|_| "{}".to_string());
                            Self::push_sse_event(&mut out, "inbox", &s);
                        }
                    }
                    Err(e) => {
                        let s = serde_json::to_string(&serde_json::json!({
                            "schema_version": 1,
                            "error": { "code": "inbox_error", "message": e.to_string() }
                        }))
                        .unwrap_or_else(|_| "{}".to_string());
                        Self::push_sse_event(&mut out, "error", &s);
                        self.done = true;
                        self.buf = std::io::Cursor::new(out);
                        self.last_emit_at = std::time::Instant::now();
                        return Ok(());
                    }
                },
                Err(e) => {
                    let s = serde_json::to_string(&serde_json::json!({
                        "schema_version": 1,
                        "error": { "code": "inbox_error", "message": e.to_string() }
                    }))
                    .unwrap_or_else(|_| "{}".to_string());
                    Self::push_sse_event(&mut out, "error", &s);
                    self.done = true;
                    self.buf = std::io::Cursor::new(out);
                    self.last_emit_at = std::time::Instant::now();
                    return Ok(());
                }
            }

            if !out.is_empty() {
                self.buf = std::io::Cursor::new(out);
                self.last_emit_at = std::time::Instant::now();
                return Ok(());
            }

            if self.last_emit_at.elapsed() >= std::time::Duration::from_millis(self.heartbeat_ms) {
                let mut hb = Vec::new();
                Self::push_heartbeat(&mut hb);
                self.buf = std::io::Cursor::new(hb);
                self.last_emit_at = std::time::Instant::now();
                return Ok(());
            }

            if self.shutdown.load(Ordering::Relaxed) {
                self.done = true;
                self.buf = std::io::Cursor::new(Vec::new());
                return Ok(());
            }

            std::thread::sleep(std::time::Duration::from_millis(self.poll_ms));
        }
    }
}

impl Read for SseSessionStream {
    fn read(&mut self, out_buf: &mut [u8]) -> std::io::Result<usize> {
        if self.shutdown.load(Ordering::Relaxed) {
            self.done = true;
            return Ok(0);
        }

        if self.buf.position() as usize >= self.buf.get_ref().len() {
            self.fill_next()?;
        }

        // If still empty, we're done.
        if self.buf.get_ref().is_empty() {
            return Ok(0);
        }

        self.buf.read(out_buf)
    }
}

pub(crate) fn handle_session_events_sse_request(
    req: tiny_http::Request,
    path: String,
    query: String,
    cors_allow_origin: Option<String>,
    fallback_cas_root: Option<std::path::PathBuf>,
    shutdown: Arc<AtomicBool>,
) {
    let cors_allow_origin = cors_allow_origin.as_deref();
    let fallback_cas_root = fallback_cas_root.as_deref();

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 5 {
        let _ = req.respond(error_response(
            StatusCode(404),
            "not_found",
            "Invalid session route",
            cors_allow_origin,
        ));
        return;
    }
    let session_name = parts[3];

    let session = match resolve_session_by_name(session_name) {
        Ok(s) => s,
        Err(e) => {
            let _ = req.respond(error_response(
                StatusCode(404),
                "session_not_found",
                e.to_string(),
                cors_allow_origin,
            ));
            return;
        }
    };
    let cas_root = match cas_root_for_session_with_fallback(&session, fallback_cas_root) {
        Ok(p) => p,
        Err(e) => {
            let _ = req.respond(error_response(
                StatusCode(500),
                "cas_root_error",
                e.to_string(),
                cors_allow_origin,
            ));
            return;
        }
    };

    let inbox_id = parse_string_query(&query, "inbox_id").unwrap_or_else(|| "owner".to_string());
    let poll_ms = parse_u64_query(&query, "poll_ms")
        .unwrap_or(500)
        .clamp(50, 5_000);
    let heartbeat_ms = parse_u64_query(&query, "heartbeat_ms")
        .unwrap_or(15_000)
        .clamp(250, 120_000);
    let activity_limit = parse_u64_query(&query, "activity_limit")
        .unwrap_or(50)
        .clamp(1, 200) as usize;
    let inbox_limit = parse_u64_query(&query, "inbox_limit")
        .unwrap_or(25)
        .clamp(1, 200) as usize;
    let status_interval_ms = parse_u64_query(&query, "status_interval_ms")
        .unwrap_or(2_000)
        .clamp(0, 120_000);
    let include_status = parse_bool_query(&query, "include_status").unwrap_or(true);
    let status_interval_ms = if include_status {
        status_interval_ms
    } else {
        0
    };
    let since_id = parse_u64_query(&query, "since_id").unwrap_or(0) as i64;

    let mut stream = SseSessionStream::new(
        cas_root,
        session,
        inbox_id,
        shutdown,
        poll_ms,
        heartbeat_ms,
        activity_limit,
        inbox_limit,
        status_interval_ms,
        since_id,
    );

    // NOTE: We cannot use `req.respond(Response::new(...))` for long-lived SSE because
    // tiny_http flushes the writer only after the body is fully written. For a never-ending
    // stream, that means clients would never receive the status line/headers.
    let mut w = req.into_writer();

    let _ = (|| -> std::io::Result<()> {
        write!(w, "HTTP/1.1 200 OK\r\n")?;
        w.write_all(b"Content-Type: text/event-stream\r\n")?;
        w.write_all(b"Cache-Control: no-cache\r\n")?;
        w.write_all(b"X-Accel-Buffering: no\r\n")?;
        w.write_all(b"Transfer-Encoding: chunked\r\n")?;
        w.write_all(b"Connection: keep-alive\r\n")?;
        if let Some(origin) = cors_allow_origin {
            write!(w, "Access-Control-Allow-Origin: {origin}\r\n")?;
            w.write_all(b"Access-Control-Allow-Headers: authorization, content-type\r\n")?;
            w.write_all(b"Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n")?;
        }
        w.write_all(b"\r\n")?;
        w.flush()
    })();

    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if write_chunked(&mut w, &buf[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = write_chunked_end(&mut w);
}
