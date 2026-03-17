use crate::ui::factory::daemon::imports::*;

impl FactoryDaemon {
    pub fn new(config: DaemonConfig) -> anyhow::Result<Self> {
        // Get initial terminal size (default for daemon without terminal)
        let (cols, rows) = (120, 40);

        // Extract fields before factory_config is moved
        let project_dir = config.factory_config.cwd.to_string_lossy().to_string();
        let lead_session_id = config.factory_config.lead_session_id.clone();

        // Set factory session env var so PTY children (and their MCP servers) inherit it.
        // SAFETY: called before spawning any threads or async tasks in this process.
        unsafe { std::env::set_var("CAS_FACTORY_SESSION", &config.session_name) };

        // Create the factory app (this spawns Claude instances)
        let mut app = FactoryApp::new(config.factory_config)?;
        app.set_factory_session(config.session_name.clone());

        // Track factory session start
        crate::telemetry::track_factory_started("supervisor", app.worker_names().len());
        let initial_workers = app.worker_names().len().to_string();
        crate::telemetry::track(
            "factory_session_started",
            vec![
                ("mode", "daemon"),
                (
                    "worktrees_enabled",
                    if app.worktrees_enabled() {
                        "true"
                    } else {
                        "false"
                    },
                ),
                ("initial_workers", &initial_workers),
            ],
        );

        // Create socket
        let sock_path = socket_path(&config.session_name);

        // Remove stale socket if it exists
        if sock_path.exists() {
            std::fs::remove_file(&sock_path)?;
        }

        // Ensure parent directory exists
        if let Some(parent) = sock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Create listener
        let listener = UnixListener::bind(&sock_path)?;
        listener.set_nonblocking(true)?;

        // Create GUI socket (for desktop GUI clients using JSON protocol)
        let gui_sock_path = gui_socket_path(&config.session_name);
        if gui_sock_path.exists() {
            std::fs::remove_file(&gui_sock_path)?;
        }
        let gui_listener = UnixListener::bind(&gui_sock_path)?;
        gui_listener.set_nonblocking(true)?;

        let session_manager = SessionManager::new();

        // Optionally start cloud phone-home client
        let cloud_handle = if config.phone_home {
            Self::try_start_cloud_client(&config.session_name)
        } else {
            None
        };

        // Remove orphaned team directories from previous crashed sessions
        super::teams::TeamsManager::cleanup_orphans();

        // Initialize native Agent Teams for inter-agent messaging (Claude CLI only).
        let teams = {
            let tm = super::teams::TeamsManager::new(&config.session_name);
            let worker_cwds: std::collections::HashMap<String, std::path::PathBuf> = app
                .worktree_manager()
                .map(|mgr| {
                    app.worker_names()
                        .iter()
                        .map(|name| (name.clone(), mgr.worktree_path_for_worker(name)))
                        .collect()
                })
                .unwrap_or_default();
            let lead_sid = lead_session_id.as_deref().unwrap_or(&config.session_name);
            match tm.init_team_config(
                app.worker_names(),
                app.project_path(),
                &worker_cwds,
                lead_sid,
            ) {
                Ok(()) => Some(tm),
                Err(e) => {
                    tracing::error!("Failed to init Teams config: {}", e);
                    None
                }
            }
        };

        // Bind notification socket for instant prompt queue wakeup
        let notify_rx = match cas_factory::DaemonNotifier::bind(app.cas_dir()) {
            Ok(n) => Some(n),
            Err(e) => {
                tracing::warn!(
                    "Failed to create notification socket, falling back to polling: {}",
                    e
                );
                None
            }
        };

        // Save session metadata (after teams init so team_name is included)
        let mut metadata = create_metadata(
            &config.session_name,
            std::process::id(),
            app.supervisor_name(),
            app.worker_names(),
            app.epic_state().epic_id(),
            Some(&project_dir),
            None, // ws_port - Unix socket daemon doesn't use WebSocket
        );
        metadata.team_name = teams.as_ref().map(|t| t.team_name().to_string());
        session_manager.save_metadata(&metadata)?;

        Ok(Self {
            session_name: config.session_name,
            app,
            listener,
            clients: HashMap::new(),
            next_client_id: 0,
            owner_client_id: None,
            owner_last_activity: Instant::now(),
            session_manager,
            shutdown: Arc::new(AtomicBool::new(false)),
            cols,
            rows,
            pending_resize: None,
            pending_resize_at: Instant::now(),
            compact_terminal: None,
            compact_cols: 0,
            compact_rows: 0,
            pending_spawns: VecDeque::new(),
            spawn_task: None,
            cloud_handle,
            relay_clients: HashMap::new(),
            pane_watchers: HashMap::new(),
            pane_buffers: HashMap::new(),
            gui_listener,
            gui_clients: HashMap::new(),
            next_gui_client_id: 0,
            tui_pane_sizes: HashMap::new(),
            web_pane_sizes: HashMap::new(),
            teams,
            notify_rx,
        })
    }

    /// Get the shutdown flag for external control
    pub fn shutdown_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }

    /// Run the daemon main loop with TUI rendering
    pub async fn run(&mut self) -> anyhow::Result<()> {
        let session_started_at = Instant::now();

        // Create buffer backend for rendering
        let backend = BufferBackend::new(self.cols, self.rows);
        let mut terminal = Terminal::new(backend)?;

        // Set initial terminal title
        set_terminal_title(self.app.project_path(), self.app.epic_state().epic_title());

        // Start recording if enabled
        if self.app.record_enabled() {
            if let Err(e) = self.app.start_recording().await {
                tracing::error!("Failed to start recording: {}", e);
            }
        }

        // Refresh intervals
        let mut last_refresh = std::time::Instant::now();
        let mut last_prompt_poll = std::time::Instant::now();
        let mut last_spawn_poll = std::time::Instant::now();
        let refresh_interval = Duration::from_secs(2);
        let poll_interval = Duration::from_millis(100);

        let mut prompt_notified = false;

        while !self.shutdown.load(Ordering::Relaxed) {
            // Error timeout must run in daemon mode too (not only local event loop path).
            let had_error = self.app.error_message.is_some();
            self.app.check_error_timeout();
            let error_cleared_by_timeout = had_error && self.app.error_message.is_none();

            // Accept new client connections (non-blocking)
            let new_clients = self.accept_clients()?;

            // Accept new GUI client connections (non-blocking)
            let new_gui_clients = self.accept_gui_clients();

            // Read and process input from clients
            let input_activity = self.process_client_input().await?;

            // Read and process input from GUI clients
            let gui_activity = self.process_gui_client_input().await;

            // Poll PTYs for output using coalesced batch drain (efficient for 6 Claudes generating)
            let (bytes_processed, events) = self.app.mux.poll_batch();
            let had_output = bytes_processed > 0;
            for event in events {
                self.handle_mux_event(event);
            }

            // Process relay events from cloud (remote terminal attach/input/detach)
            self.process_relay_events().await;

            // Poll prompt queue (on notification or timer)
            if prompt_notified || last_prompt_poll.elapsed() >= poll_interval {
                if prompt_notified {
                    if let Some(ref mut notify) = self.notify_rx {
                        notify.drain();
                    }
                }
                let _ = self.process_prompt_queue().await;
                last_prompt_poll = std::time::Instant::now();
                prompt_notified = false;
            }

            // Poll spawn queue (enqueues requests, doesn't execute them)
            if last_spawn_poll.elapsed() >= poll_interval {
                let _ = self.enqueue_spawn_requests();
                last_spawn_poll = std::time::Instant::now();
            }

            // Process pending spawns (non-blocking: git ops run on background thread)
            if self.spawn_task.is_some() || !self.pending_spawns.is_empty() {
                self.process_pending_spawns().await;
            }

            // Periodic CAS data refresh
            let mut refreshed = false;
            if last_refresh.elapsed() >= refresh_interval {
                if let Ok((prompts, events)) = self.app.refresh_data() {
                    // Record events for export
                    self.app.record_events(&events);

                    // Send notifications for detected events
                    self.app.notify_events(&events);

                    // Handle epic state transitions
                    let changes = self.app.handle_epic_events(&events);
                    for change in changes {
                        let _ = self.handle_epic_change(change).await;
                    }

                    // Process reminders (time-based and event-based)
                    self.process_reminders(&events);

                    // Push state and events to cloud (best-effort, no-op if not connected)
                    self.push_cloud_events(&events);
                    self.push_cloud_state();

                    // Inject prompts (config already checked in generate_prompt)
                    for prompt in prompts {
                        if let Some(ref teams) = self.teams {
                            let _ = teams.write_to_inbox(
                                &prompt.target,
                                super::teams::DIRECTOR_AGENT_NAME,
                                &prompt.text,
                                None,
                                None,
                            );
                        } else {
                            let _ = self.app.mux.inject(&prompt.target, &prompt.text).await;
                        }
                    }
                }
                last_refresh = std::time::Instant::now();
                refreshed = true;
            }

            // Apply debounced resize after 100ms of no new resize events
            let mut resize_applied = false;
            if let Some((cols, rows)) = self.pending_resize {
                if self.pending_resize_at.elapsed() >= Duration::from_millis(100) {
                    tracing::info!("Applying debounced resize: {}x{}", cols, rows);

                    // Determine if we have full-mode clients that need the full layout resize
                    let has_full_clients = self
                        .clients
                        .values()
                        .any(|c| c.view_mode == ClientViewMode::Full);

                    if has_full_clients {
                        // Use the largest full-mode client dimensions for the full layout
                        let (full_cols, full_rows) = self.dims_for_mode(ClientViewMode::Full);
                        if full_cols > 0 && full_rows > 0 {
                            self.cols = full_cols;
                            self.rows = full_rows;
                            let _ = self.app.handle_resize(full_cols, full_rows);
                        }
                    } else if cols >= COMPACT_WIDTH_THRESHOLD {
                        // No explicit full clients but this resize is full-sized
                        self.cols = cols;
                        self.rows = rows;
                        let _ = self.app.handle_resize(cols, rows);
                    }

                    // Update compact terminal dimensions if compact clients exist
                    let has_compact_clients = self
                        .clients
                        .values()
                        .any(|c| c.view_mode == ClientViewMode::Compact);

                    if has_compact_clients {
                        let (cc, cr) = self.dims_for_mode(ClientViewMode::Compact);
                        if cc > 0 && cr > 0 && (cc != self.compact_cols || cr != self.compact_rows)
                        {
                            self.compact_cols = cc;
                            self.compact_rows = cr;
                            // Resize supervisor PTY to fit compact layout if no full clients
                            // are connected (phone is the only viewer)
                            if !has_full_clients {
                                let sup_rows = cr.saturating_sub(1); // 1 for status bar
                                let sup_cols = cc;
                                let sup_name = self.app.supervisor_name().to_string();
                                if let Some(pane) = self.app.mux.get_mut(&sup_name) {
                                    let _ = pane.resize(sup_rows, sup_cols);
                                }
                            }
                            // Rebuild compact terminal
                            let backend = BufferBackend::new(cc, cr);
                            self.compact_terminal = Some(Terminal::new(backend)?);
                        }
                    }

                    // Snapshot TUI pane sizes and reconcile with GUI/web
                    // constraints (smallest client wins per pane).
                    self.snapshot_tui_pane_sizes_and_reconcile();

                    // Rebuild pane ring buffers from the virtual terminal's
                    // current state — after resize the vt reflows content to
                    // the new dimensions, so we snapshot that and re-encode
                    // as ANSI bytes. This preserves history for web viewers.
                    self.rebuild_pane_buffers_from_snapshots();

                    self.pending_resize = None;
                    resize_applied = true;
                }
            }

            // Check if full-mode clients need a full redraw
            let needs_full_redraw = self
                .clients
                .values()
                .any(|c| c.view_mode == ClientViewMode::Full && c.needs_full_redraw);
            if needs_full_redraw || resize_applied {
                // Resize the backend to match current dimensions
                terminal.backend_mut().resize(self.cols, self.rows);
                terminal.autoresize()?;
                // When a client needs a full redraw (new connection, buffer overflow),
                // reset the terminal's diff state so the next draw() produces a
                // complete frame. Without this, autoresize() is a no-op when dims
                // haven't changed, and draw() emits only a diff against the previous
                // frame—which the new client doesn't have (its screen is blank).
                if needs_full_redraw {
                    terminal.clear()?;
                }
                for client in self.clients.values_mut() {
                    if client.view_mode == ClientViewMode::Full {
                        client.needs_full_redraw = false;
                    }
                }
            }

            // Check if compact clients need a full redraw
            let needs_compact_redraw = self
                .clients
                .values()
                .any(|c| c.view_mode == ClientViewMode::Compact && c.needs_full_redraw);
            if needs_compact_redraw || resize_applied {
                if let Some(ref mut ct) = self.compact_terminal {
                    ct.backend_mut()
                        .resize(self.compact_cols, self.compact_rows);
                    ct.autoresize()?;
                    if needs_compact_redraw {
                        ct.clear()?;
                    }
                }
                for client in self.clients.values_mut() {
                    if client.view_mode == ClientViewMode::Compact {
                        client.needs_full_redraw = false;
                    }
                }
            }

            // Suppress rendering while a resize is pending (debouncing).
            // Rendering at the old size while the terminal is already a new size
            // produces visual garbage. Wait for the debounce to settle.
            let resize_pending = self.pending_resize.is_some();

            // Send periodic state updates to GUI clients on refresh
            if refreshed && !self.gui_clients.is_empty() {
                self.gui_send_state_update();
            }

            let spawning = self.app.spawning_count > 0;
            let dirty = had_output
                || input_activity
                || gui_activity
                || refreshed
                || new_clients
                || new_gui_clients
                || needs_full_redraw
                || needs_compact_redraw
                || resize_applied
                || spawning
                || error_cleared_by_timeout;
            if dirty && !resize_pending {
                // Render full TUI for full-mode clients (and relay clients)
                let has_full_clients = self
                    .clients
                    .values()
                    .any(|c| c.view_mode == ClientViewMode::Full);
                let has_relay = self.has_relay_clients();
                if has_full_clients || has_relay {
                    terminal.draw(|f| self.app.render(f))?;
                    let output = terminal.backend_mut().take_buffer();
                    if !output.is_empty() {
                        if has_full_clients {
                            self.broadcast_output_to(&output, ClientViewMode::Full);
                        }
                        if has_relay {
                            self.broadcast_relay_output(&output);
                        }
                    }
                }

                // Render compact TUI for compact-mode clients
                let has_compact_clients = self
                    .clients
                    .values()
                    .any(|c| c.view_mode == ClientViewMode::Compact);
                if has_compact_clients {
                    if let Some(ref mut ct) = self.compact_terminal {
                        ct.draw(|f| self.app.render_compact(f))?;
                        let output = ct.backend_mut().take_buffer();
                        if !output.is_empty() {
                            self.broadcast_output_to(&output, ClientViewMode::Compact);
                        }
                    }
                }
            }
            // Flush any pending client output even if nothing new was rendered
            if !self.clients.is_empty() && self.clients.values().any(|c| !c.output_buf.is_empty()) {
                self.flush_client_output();
            }

            // Flush pending GUI client output
            if !self.gui_clients.is_empty()
                && self.gui_clients.values().any(|c| !c.write_buf.is_empty())
            {
                self.flush_gui_client_output();
            }

            // Adaptive sleep: ~120fps when active, ~60fps when idle, ~10fps for spinner
            let sleep_ms = if had_output {
                4
            } else if spawning {
                100 // Spinner updates every 100ms
            } else if !self.clients.is_empty() {
                8
            } else {
                16
            };
            let sleep_dur = Duration::from_millis(sleep_ms);
            if let Some(ref mut notify) = self.notify_rx {
                tokio::select! {
                    result = notify.recv() => {
                        if result.is_ok() {
                            prompt_notified = true;
                        }
                    }
                    _ = tokio::time::sleep(sleep_dur) => {}
                }
            } else {
                tokio::time::sleep(sleep_dur).await;
            }
        }

        // Stop recording if it was enabled
        if self.app.record_enabled() {
            if let Err(e) = self.app.stop_recording().await {
                tracing::error!("Failed to stop recording: {}", e);
            }

            // Upload recordings to cloud (best-effort, before disconnect)
            self.upload_recordings();
        }

        // Cleanup
        let cleanup_result = self.cleanup();

        let duration_secs = session_started_at.elapsed().as_secs().to_string();
        let final_workers = self.app.worker_names().len().to_string();
        crate::telemetry::track(
            "factory_session_ended",
            vec![
                ("mode", "daemon"),
                (
                    "status",
                    if cleanup_result.is_ok() {
                        "ok"
                    } else {
                        "error"
                    },
                ),
                ("duration_secs", &duration_secs),
                ("final_workers", &final_workers),
            ],
        );

        cleanup_result?;

        Ok(())
    }

    /// Cleanup on shutdown
    fn cleanup(&mut self) -> anyhow::Result<()> {
        // Clean up notification socket
        if let Some(ref notify) = self.notify_rx {
            notify.cleanup();
        }

        // Clean up native Agent Teams directory
        if let Some(ref teams) = self.teams {
            teams.cleanup();
        }

        // Disconnect cloud phone-home client
        self.disconnect_cloud();

        // Kill all PTY processes (Claude instances)
        self.app.mux.kill_all();

        // Unregister all factory agents (supervisor + workers)
        if let Ok(agent_store) = open_agent_store(self.app.cas_dir()) {
            // Collect all agent names to unregister
            let mut names_to_unregister = vec![self.app.supervisor_name().to_string()];
            names_to_unregister.extend(self.app.worker_names().iter().cloned());

            // Query agent store directly instead of using cached director_data
            // to ensure we find all agents, including those registered after cache refresh
            if let Ok(all_agents) = agent_store.list(None) {
                for name in &names_to_unregister {
                    if let Some(agent) = all_agents.iter().find(|a| a.name == *name) {
                        if let Err(e) = agent_store.unregister(&agent.id) {
                            tracing::warn!("Failed to unregister agent {}: {}", name, e);
                        }
                    }
                }
            }
        }

        // Remove session metadata
        self.session_manager.remove_metadata(&self.session_name)?;

        // Clean up GUI socket
        let gui_sock = gui_socket_path(&self.session_name);
        let _ = std::fs::remove_file(&gui_sock);

        // Send leave alternate screen to all clients
        let cleanup = b"\x1b[?25h\x1b[?1049l";
        for client in self.clients.values_mut() {
            let _ = client.stream.write_all(cleanup);
        }

        Ok(())
    }
}
