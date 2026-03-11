use std::collections::VecDeque;
use std::fmt::Write as FmtWrite;

use cas_factory_protocol::{
    STYLE_BOLD, STYLE_FAINT, STYLE_INVERSE, STYLE_INVISIBLE, STYLE_ITALIC, STYLE_STRIKETHROUGH,
    STYLE_UNDERLINE, TerminalSnapshot,
};

use crate::ui::factory::daemon::FactoryDaemon;
use crate::ui::factory::daemon::cloud_client::RelayEvent;

/// Max bytes to keep per pane for replay on attach (256 KB).
const PANE_BUFFER_CAPACITY: usize = 256 * 1024;

/// Tracks a remote relay client connected via cloud WebSocket
#[derive(Debug)]
pub struct RelayClient {
    /// Terminal columns
    pub cols: u16,
    /// Terminal rows
    pub rows: u16,
    /// Whether this client can send input (vs read-only)
    pub interactive: bool,
}

/// Ring buffer of raw PTY bytes for a single pane.
///
/// When a web viewer attaches, we replay this buffer so xterm.js
/// processes the same byte stream and ends up in the identical
/// terminal state (alternate screen, colors, cursor, etc.).
#[derive(Debug, Default)]
pub struct PaneBuffer {
    data: VecDeque<u8>,
}

impl PaneBuffer {
    pub(in crate::ui::factory::daemon) fn append(&mut self, bytes: &[u8]) {
        self.data.extend(bytes);
        // Trim from the front if over capacity
        if self.data.len() > PANE_BUFFER_CAPACITY {
            let excess = self.data.len() - PANE_BUFFER_CAPACITY;
            self.data.drain(..excess);
        }
    }

    pub(in crate::ui::factory::daemon) fn as_bytes(&self) -> Vec<u8> {
        self.data.iter().copied().collect()
    }

    pub(in crate::ui::factory::daemon) fn replace_with(&mut self, bytes: Vec<u8>) {
        self.data.clear();
        self.data.extend(bytes);
        if self.data.len() > PANE_BUFFER_CAPACITY {
            let excess = self.data.len() - PANE_BUFFER_CAPACITY;
            self.data.drain(..excess);
        }
    }
}

/// Render a `TerminalSnapshot` as raw ANSI escape sequences.
///
/// After a PTY resize the virtual terminal reflows content to the new
/// dimensions. This function captures that reflowed viewport as ANSI bytes
/// suitable for replaying into a web terminal emulator (ghostty-web / xterm.js),
/// preserving colors, bold/italic, and cursor position.
fn snapshot_to_ansi(snap: &TerminalSnapshot) -> Vec<u8> {
    let cap = (snap.rows as usize) * (snap.cols as usize) * 4;
    let mut buf = Vec::with_capacity(cap);
    let mut tmp = String::new();

    // Reset attributes, clear screen, cursor home
    buf.extend_from_slice(b"\x1b[0m\x1b[2J\x1b[H");

    let mut prev_fg: (u8, u8, u8) = (0, 0, 0);
    let mut prev_bg: (u8, u8, u8) = (0, 0, 0);
    let mut prev_flags: u32 = 0;

    for row in 0..snap.rows {
        // Move cursor to start of row
        tmp.clear();
        let _ = write!(tmp, "\x1b[{};1H", row + 1);
        buf.extend_from_slice(tmp.as_bytes());

        for col in 0..snap.cols {
            let idx = (row as usize) * (snap.cols as usize) + (col as usize);
            let cell = &snap.cells[idx];

            // Emit SGR sequence when style changes
            if cell.fg != prev_fg || cell.bg != prev_bg || cell.flags != prev_flags {
                emit_sgr(&mut buf, cell.fg, cell.bg, cell.flags);
                prev_fg = cell.fg;
                prev_bg = cell.bg;
                prev_flags = cell.flags;
            }

            // Write character
            let ch = char::from_u32(cell.codepoint).unwrap_or(' ');
            let mut char_buf = [0u8; 4];
            buf.extend_from_slice(ch.encode_utf8(&mut char_buf).as_bytes());
        }
    }

    // Reset attributes, then position cursor
    buf.extend_from_slice(b"\x1b[0m");
    tmp.clear();
    let _ = write!(tmp, "\x1b[{};{}H", snap.cursor.y + 1, snap.cursor.x + 1);
    buf.extend_from_slice(tmp.as_bytes());

    buf
}

/// Emit an SGR (Select Graphic Rendition) escape sequence into `buf`.
fn emit_sgr(buf: &mut Vec<u8>, fg: (u8, u8, u8), bg: (u8, u8, u8), flags: u32) {
    // Start with reset so each SGR is self-contained
    let mut params = String::from("\x1b[0");

    if flags & STYLE_BOLD != 0 {
        params.push_str(";1");
    }
    if flags & STYLE_FAINT != 0 {
        params.push_str(";2");
    }
    if flags & STYLE_ITALIC != 0 {
        params.push_str(";3");
    }
    if flags & STYLE_UNDERLINE != 0 {
        params.push_str(";4");
    }
    if flags & STYLE_INVERSE != 0 {
        params.push_str(";7");
    }
    if flags & STYLE_INVISIBLE != 0 {
        params.push_str(";8");
    }
    if flags & STYLE_STRIKETHROUGH != 0 {
        params.push_str(";9");
    }

    // 24-bit fg color (skip if default black)
    if fg != (0, 0, 0) {
        let _ = write!(params, ";38;2;{};{};{}", fg.0, fg.1, fg.2);
    }
    // 24-bit bg color (skip if default black)
    if bg != (0, 0, 0) {
        let _ = write!(params, ";48;2;{};{};{}", bg.0, bg.1, bg.2);
    }

    params.push('m');
    buf.extend_from_slice(params.as_bytes());
}

impl FactoryDaemon {
    /// Process pending relay events from the cloud client.
    ///
    /// Called each tick in the main daemon loop. Handles attach/detach/input/resize
    /// from remote users connected via the cloud relay channel.
    pub(super) async fn process_relay_events(&mut self) {
        let events = match self.cloud_handle {
            Some(ref handle) => handle.try_recv_relay(),
            None => return,
        };

        for event in events {
            match event {
                RelayEvent::AttachRequest {
                    client_id,
                    cols,
                    rows,
                    mode,
                } => {
                    self.handle_relay_attach(&client_id, cols, rows, &mode);
                }
                RelayEvent::PtyInput { client_id, data } => {
                    self.handle_relay_input(&client_id, &data).await;
                }
                RelayEvent::Resize {
                    client_id,
                    cols,
                    rows,
                } => {
                    self.handle_relay_resize(&client_id, cols, rows);
                }
                RelayEvent::Detach { client_id } => {
                    self.handle_relay_detach(&client_id);
                }
                RelayEvent::PaneAttach { pane, cols, rows } => {
                    self.handle_pane_attach(&pane, cols, rows);
                }
                RelayEvent::PaneResize { pane, cols, rows } => {
                    self.handle_pane_resize(&pane, cols, rows);
                }
                RelayEvent::PaneDetach { pane } => {
                    self.handle_pane_detach(&pane);
                }
                RelayEvent::PaneInput { pane, data } => {
                    self.handle_pane_input(&pane, &data).await;
                }
            }
        }
    }

    fn handle_relay_attach(&mut self, client_id: &str, cols: u16, rows: u16, mode: &str) {
        let interactive = mode != "readonly";

        let client = RelayClient {
            cols,
            rows,
            interactive,
        };

        tracing::info!(
            "Relay client {} attached ({}x{}, {})",
            client_id,
            cols,
            rows,
            if interactive {
                "interactive"
            } else {
                "readonly"
            }
        );

        self.relay_clients.insert(client_id.to_string(), client);

        // Accept the attach request
        if let Some(ref handle) = self.cloud_handle {
            handle.relay_accept(client_id, cols, rows);
        }
    }

    async fn handle_relay_input(&mut self, client_id: &str, data: &[u8]) {
        let is_interactive = self
            .relay_clients
            .get(client_id)
            .is_some_and(|c| c.interactive);

        if !is_interactive {
            return;
        }

        // Forward input to the focused pane (same as local client input)
        let _ = self.app.mux.send_input(data).await;
    }

    fn handle_relay_resize(&mut self, client_id: &str, cols: u16, rows: u16) {
        if let Some(client) = self.relay_clients.get_mut(client_id) {
            client.cols = cols;
            client.rows = rows;
            tracing::debug!("Relay client {} resized to {}x{}", client_id, cols, rows);
        }
    }

    fn handle_relay_detach(&mut self, client_id: &str) {
        if self.relay_clients.remove(client_id).is_some() {
            tracing::info!("Relay client {} detached", client_id);
        }
    }

    /// Broadcast rendered TUI output to all relay clients.
    ///
    /// Called after rendering, sends the full terminal output frame to each
    /// relay client via the cloud WebSocket.
    pub(super) fn broadcast_relay_output(&self, output: &[u8]) {
        if self.relay_clients.is_empty() || output.is_empty() {
            return;
        }
        if let Some(ref handle) = self.cloud_handle {
            for client_id in self.relay_clients.keys() {
                handle.relay_output(client_id, output.to_vec());
            }
        }
    }

    /// Check if any relay clients are connected
    pub(super) fn has_relay_clients(&self) -> bool {
        !self.relay_clients.is_empty()
    }

    // --- Per-pane relay (web terminal viewers) ---

    /// Buffer raw PTY bytes for a pane. Called for EVERY PaneOutput event,
    /// regardless of whether anyone is watching. This ensures the buffer
    /// is warm when a viewer attaches.
    pub(super) fn buffer_pane_output(&mut self, pane_id: &str, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.pane_buffers
            .entry(pane_id.to_string())
            .or_default()
            .append(data);
    }

    /// Rebuild all pane ring buffers from current terminal snapshots.
    ///
    /// Called after a PTY resize so the buffers contain ANSI bytes
    /// rendered at the new dimensions (the virtual terminal reflows
    /// content on resize). Without this, the buffer would hold a mix
    /// of old-size and new-size bytes which looks garbled.
    pub(super) fn rebuild_pane_buffers_from_snapshots(&mut self) {
        let pane_ids: Vec<String> = self.pane_buffers.keys().cloned().collect();
        for pane_id in pane_ids {
            if let Some(pane) = self.app.mux.get(&pane_id) {
                if let Ok(snapshot) = pane.get_full_snapshot() {
                    let ansi = snapshot_to_ansi(&snapshot);
                    if let Some(buffer) = self.pane_buffers.get_mut(&pane_id) {
                        buffer.replace_with(ansi);
                    }
                }
            }
        }
    }

    fn handle_pane_attach(&mut self, pane: &str, cols: Option<u16>, rows: Option<u16>) {
        let watchers = self.pane_watchers.entry(pane.to_string()).or_default();
        let was_empty = watchers.is_empty();
        watchers.insert("web".to_string());

        if was_empty {
            tracing::info!(
                "Pane '{}' now has web watchers, starting output relay",
                pane
            );

            // Send the pane list so the frontend knows available panes
            if let Some(ref handle) = self.cloud_handle {
                let mut panes = self.app.worker_names().to_vec();
                panes.insert(0, self.app.supervisor_name().to_string());
                handle.send_pane_list(panes);
            }
        }

        let actual_pane = if pane == "supervisor" {
            self.app.supervisor_name().to_string()
        } else {
            pane.to_string()
        };

        // If the web viewer sent dimensions, resize the PTY to match so
        // output is formatted for the viewer's terminal size.
        if let (Some(c), Some(r)) = (cols, rows) {
            if let Some(mux_pane) = self.app.mux.get_mut(&actual_pane) {
                let (old_rows, old_cols) = mux_pane.size();
                if old_cols != c || old_rows != r {
                    match mux_pane.resize(r, c) {
                        Ok(()) => {
                            tracing::info!(
                                "Resized pane '{}' from {}x{} to {}x{} for web viewer",
                                pane,
                                old_cols,
                                old_rows,
                                c,
                                r
                            );
                            // Rebuild the buffer from the vt snapshot at the
                            // new dimensions (the virtual terminal reflows on
                            // resize so the snapshot has correct content).
                            if let Some(p) = self.app.mux.get(&actual_pane) {
                                if let Ok(snapshot) = p.get_full_snapshot() {
                                    let ansi = snapshot_to_ansi(&snapshot);
                                    self.pane_buffers
                                        .entry(actual_pane.clone())
                                        .or_default()
                                        .replace_with(ansi);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to resize pane '{}': {}", pane, e);
                        }
                    }
                }
            }
        }

        // Replay buffered PTY bytes so the browser's terminal emulator ends up
        // in the same terminal state (alternate screen, colors, cursor, etc.).
        if let Some(buffer) = self.pane_buffers.get(&actual_pane) {
            let replay = buffer.as_bytes();
            if !replay.is_empty() {
                tracing::info!(
                    "Replaying {} bytes of PTY history for pane '{}'",
                    replay.len(),
                    pane
                );
                if let Some(ref handle) = self.cloud_handle {
                    handle.send_pane_output(pane, replay);
                }
            }
        }
    }

    fn handle_pane_resize(&mut self, pane: &str, cols: u16, rows: u16) {
        let actual_pane = if pane == "supervisor" {
            self.app.supervisor_name().to_string()
        } else {
            pane.to_string()
        };

        // Store web viewer's size and apply unified effective minimum
        self.web_pane_sizes
            .insert(actual_pane.clone(), (cols, rows));
        self.apply_effective_pane_size(&actual_pane);

        // Rebuild the buffer from the reflowed vt snapshot
        if let Some(p) = self.app.mux.get(&actual_pane) {
            if let Ok(snapshot) = p.get_full_snapshot() {
                let ansi = snapshot_to_ansi(&snapshot);
                self.pane_buffers
                    .entry(actual_pane)
                    .or_default()
                    .replace_with(ansi);
            }
        }
    }

    fn handle_pane_detach(&mut self, pane: &str) {
        let actual_pane = if pane == "supervisor" {
            self.app.supervisor_name().to_string()
        } else {
            pane.to_string()
        };

        if let Some(watchers) = self.pane_watchers.get_mut(pane) {
            watchers.remove("web");
            if watchers.is_empty() {
                self.pane_watchers.remove(pane);
                self.web_pane_sizes.remove(&actual_pane);
                tracing::info!("Pane '{}' has no more web watchers", pane);
            }
        }

        // When no web viewers remain on ANY pane, clear all web sizes
        // and restore panes to their effective size (TUI/GUI constraints).
        if self.pane_watchers.is_empty() {
            self.web_pane_sizes.clear();
            if self.cols > 0 && self.rows > 0 {
                tracing::info!(
                    "No web viewers remain, restoring local terminal size {}x{}",
                    self.cols,
                    self.rows
                );
                let _ = self.app.handle_resize(self.cols, self.rows);
                self.snapshot_tui_pane_sizes_and_reconcile();
                self.rebuild_pane_buffers_from_snapshots();
            }
        } else {
            // Recalculate this pane's effective size without the web constraint
            self.apply_effective_pane_size(&actual_pane);
        }
    }

    async fn handle_pane_input(&mut self, pane: &str, data: &[u8]) {
        // Resolve "supervisor" to actual pane name
        let actual_pane = if pane == "supervisor" {
            self.app.supervisor_name().to_string()
        } else {
            pane.to_string()
        };
        let _ = self.app.mux.send_input_to(&actual_pane, data).await;
    }

    /// Forward per-pane PTY output to web watchers.
    ///
    /// Called from handle_mux_event when PaneOutput arrives.
    pub(super) fn forward_pane_output(&self, pane_id: &str, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        // Check if anyone is watching this pane (by actual name or "supervisor" alias)
        let is_watched = self.pane_watchers.contains_key(pane_id)
            || (pane_id == self.app.supervisor_name()
                && self.pane_watchers.contains_key("supervisor"));

        if is_watched {
            if let Some(ref handle) = self.cloud_handle {
                // Send with the logical name the frontend knows
                let pane_name = if pane_id == self.app.supervisor_name() {
                    "supervisor"
                } else {
                    pane_id
                };
                handle.send_pane_output(pane_name, data.to_vec());
            }
        }
    }
}
