use crate::ui::factory::app::imports::*;

fn bool_prop(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn shutdown_scope(count: Option<usize>, names: &[String]) -> &'static str {
    if !names.is_empty() {
        "named"
    } else if count.unwrap_or(0) == 0 {
        "all"
    } else {
        "count"
    }
}

impl FactoryApp {
    /// Get the current epic state
    pub fn epic_state(&self) -> &EpicState {
        &self.epic_state
    }

    /// Handle epic state transitions based on detected events
    ///
    /// Returns true if state changed (for branch management).
    pub fn handle_epic_events(&mut self, events: &[DirectorEvent]) -> Vec<EpicStateChange> {
        let mut changes = Vec::new();

        for event in events {
            match event {
                DirectorEvent::EpicStarted {
                    epic_id,
                    epic_title,
                } => {
                    // Transition to Active state
                    let previous = std::mem::replace(
                        &mut self.epic_state,
                        EpicState::Active {
                            epic_id: epic_id.clone(),
                            epic_title: epic_title.clone(),
                        },
                    );

                    changes.push(EpicStateChange::Started {
                        epic_id: epic_id.clone(),
                        epic_title: epic_title.clone(),
                        previous_state: previous,
                    });
                }

                DirectorEvent::EpicCompleted { epic_id } => {
                    // Check if this is our current epic
                    if self.epic_state.epic_id() == Some(epic_id) {
                        let title = self
                            .epic_state
                            .epic_title()
                            .unwrap_or("Unknown")
                            .to_string();

                        // Transition to Completing state
                        self.epic_state = EpicState::Completing {
                            epic_id: epic_id.clone(),
                            epic_title: title.clone(),
                        };

                        changes.push(EpicStateChange::Completed {
                            epic_id: epic_id.clone(),
                            epic_title: title,
                        });
                    }
                }

                _ => {}
            }
        }

        changes
    }

    /// Reset epic state to idle (after merge completes)
    pub fn reset_epic_state(&mut self) {
        self.epic_state = EpicState::Idle;
    }

    /// Add a new worker at runtime (synchronous - blocks during worktree creation).
    ///
    /// Creates a worktree (if isolate is true and worktrees enabled) and spawns a Claude instance.
    /// For non-blocking spawning, use `prepare_worker_spawn` + `finish_worker_spawn`.
    pub fn spawn_worker(&mut self, name: Option<&str>, isolate: bool) -> anyhow::Result<String> {
        let prep = self.prepare_worker_spawn(name, isolate)?;
        let result = match prep.run() {
            Ok(result) => result,
            Err(e) => {
                crate::telemetry::track(
                    "factory_worker_spawn_result",
                    vec![("success", "false"), ("reason", "worktree_prepare_failed")],
                );
                return Err(e);
            }
        };
        self.finish_worker_spawn(result, None)
    }

    /// Phase 1: Prepare spawn data (fast, runs on main thread).
    ///
    /// Resolves the worker name, computes paths, and returns a `WorkerSpawnPrep`
    /// that can be sent to a background thread for the slow git operations.
    ///
    /// When `isolate` is true and worktrees are configured, each worker gets its
    /// own git worktree and branch. When false, workers share the main working directory.
    pub fn prepare_worker_spawn(
        &mut self,
        name: Option<&str>,
        isolate: bool,
    ) -> anyhow::Result<WorkerSpawnPrep> {
        let spawn_type = if name.is_some() { "named" } else { "anonymous" };
        crate::telemetry::track(
            "factory_worker_spawn_requested",
            vec![
                ("spawn_type", spawn_type),
                ("worktrees_enabled", bool_prop(self.worktrees_enabled())),
                ("isolate", bool_prop(isolate)),
            ],
        );

        // Generate a unique name if not provided
        let worker_name = match name {
            Some(n) => n.to_string(),
            None => {
                let existing: std::collections::HashSet<&str> =
                    self.worker_names.iter().map(|s| s.as_str()).collect();
                let mut candidate = generate_unique(1)[0].clone();
                let mut attempts = 0;
                while existing.contains(candidate.as_str()) && attempts < 100 {
                    candidate = generate_unique(1)[0].clone();
                    attempts += 1;
                }
                candidate
            }
        };

        if self.worker_names.contains(&worker_name) {
            crate::telemetry::track(
                "factory_worker_spawn_result",
                vec![("success", "false"), ("reason", "worker_exists")],
            );
            anyhow::bail!("Worker '{worker_name}' already exists");
        }

        let worktree_info = if isolate {
            if let Some(manager) = &self.worktree_manager {
                // Verify repo has commits before trying to create worktrees
                if !manager.git().has_commits().unwrap_or(false) {
                    crate::telemetry::track(
                        "factory_worker_spawn_result",
                        vec![("success", "false"), ("reason", "repo_has_no_commits")],
                    );
                    anyhow::bail!(
                        "Repository has no commits. Please make an initial commit before spawning workers."
                    );
                }

                let worktree_path = manager.worktree_path_for_worker(&worker_name);
                let branch_name = manager.branch_name_for_worker(&worker_name);
                let repo_root = manager.repo_root().to_path_buf();
                let parent_branch = manager
                    .git()
                    .current_branch()
                    .unwrap_or_else(|_| manager.git().detect_default_branch());
                Some(WorktreePrep {
                    worktree_path,
                    branch_name,
                    parent_branch,
                    repo_root,
                    cas_dir: self.cas_dir.clone(),
                })
            } else {
                anyhow::bail!(
                    "Worker isolation requested but worktrees are not enabled. \
                     Start the factory with --worktrees to enable isolation."
                );
            }
        } else {
            None
        };

        crate::telemetry::track(
            "factory_worker_spawn_prepared",
            vec![
                ("spawn_type", spawn_type),
                ("worktrees_enabled", bool_prop(worktree_info.is_some())),
            ],
        );

        Ok(WorkerSpawnPrep {
            worker_name,
            worktree_info,
        })
    }

    /// Phase 3: Finish spawn on main thread (fast - adds pane to mux, updates tracking).
    ///
    /// `teams` provides per-worker Agent Teams CLI flags. When `Some`, the spawned
    /// agent will bootstrap with native Teams inbox polling. The daemon builds this
    /// from `TeamsManager::spawn_config_for()` for each worker individually.
    pub fn finish_worker_spawn(
        &mut self,
        result: WorkerSpawnResult,
        teams: Option<cas_mux::TeamsSpawnConfig>,
    ) -> anyhow::Result<String> {
        let worker_name = result.worker_name;
        let cwd = result.cwd;
        let cas_root = result.cas_root;

        // Register the worktree with the manager if applicable
        if let (Some(manager), Some(wt)) = (&mut self.worktree_manager, result.worktree) {
            manager.register_worktree(&worker_name, wt);
        }

        tracing::info!("Adding worker pane: {} in {:?}", worker_name, cwd);

        if let Err(e) = self.mux.add_worker(
            &worker_name,
            cwd,
            cas_root.as_ref(),
            &self.supervisor_name,
            teams.as_ref(),
        ) {
            crate::telemetry::track(
                "factory_worker_spawn_result",
                vec![("success", "false"), ("reason", "mux_add_worker_failed")],
            );
            return Err(e.into());
        }

        // Track the worker name
        self.worker_names.push(worker_name.clone());
        crate::ui::factory::app::queue_codex_worker_intro_prompt(
            self.cas_dir(),
            &worker_name,
            self.worker_cli,
        );

        // Update event detector so it recognizes this worker's events
        self.event_detector.add_worker(worker_name.clone());

        // Update pane grid for navigation
        self.pane_grid = PaneGrid::new(&self.worker_names, &self.supervisor_name, self.is_tabbed);

        // Sync pane sizes to accommodate new worker
        let _ = self.sync_pane_sizes();

        let workers_active = self.worker_names.len().to_string();
        crate::telemetry::track(
            "factory_worker_spawn_result",
            vec![("success", "true"), ("workers_active", &workers_active)],
        );

        tracing::info!("spawn_worker completed: {}", worker_name);
        Ok(worker_name)
    }

    /// Shutdown a worker by name
    ///
    /// Removes the worker pane and cleans up its clone (if any).
    ///
    /// # Arguments
    /// * `name` - Worker name to shutdown
    /// * `_force` - Reserved for compatibility; supervisor should decide shutdown safety
    pub fn shutdown_worker(&mut self, name: &str, _force: bool) -> anyhow::Result<()> {
        // Check if worker exists
        if !self.worker_names.contains(&name.to_string()) {
            anyhow::bail!("Worker '{name}' not found");
        }

        // Mark agent as shutdown in CAS first; this must succeed so supervisor sees errors
        // instead of silently leaving stale idle agents in director panels.
        let agent_store = open_agent_store(self.cas_dir())?;
        let agents = agent_store.list(None)?;
        let agent = agents.iter().find(|a| a.name == name).ok_or_else(|| {
            let known_workers: Vec<String> = agents
                .iter()
                .filter(|a| a.role == cas_types::AgentRole::Worker)
                .map(|a| a.name.clone())
                .collect();
            anyhow::anyhow!(
                "Cannot shutdown worker '{}': no exact CAS agent record found. Known worker records: {}",
                name,
                if known_workers.is_empty() {
                    "(none)".to_string()
                } else {
                    known_workers.join(", ")
                }
            )
        })?;

        if let Err(e) = agent_store.graceful_shutdown(&agent.id) {
            // Best effort fallback to stale state for consistency, but still surface original failure.
            let fallback = agent_store.mark_stale(&agent.id);
            anyhow::bail!(
                "Failed to gracefully shutdown worker '{}' (agent_id={}): {}. Fallback mark_stale: {}",
                name,
                agent.id,
                e,
                match fallback {
                    Ok(()) => "ok".to_string(),
                    Err(mark_err) => format!("failed ({mark_err})"),
                }
            );
        }

        // Remove from mux (this kills the Claude process)
        self.mux.remove_worker(name)?;

        // Remove from tracking
        self.worker_names.retain(|n| n != name);

        // Force a DB reload next refresh; relying only on mtime can miss rapid same-second writes.
        self.last_db_fingerprint = None;
        // Refresh director data immediately so UI shows updated state
        let _ = self.refresh_data();

        // Update event detector
        self.event_detector.remove_worker(name);

        // Update pane grid for navigation
        self.pane_grid = PaneGrid::new(&self.worker_names, &self.supervisor_name, self.is_tabbed);

        // Ensure selected tab is still valid
        self.clamp_selected_worker_tab();

        // Optionally clean up clone
        // Note: We don't delete clones by default as they may have uncommitted work

        // Sync pane sizes to adjust layout
        let _ = self.sync_pane_sizes();

        Ok(())
    }

    /// Mark a worker as crashed (removes from tracking, keeps worktree for respawn)
    ///
    /// This is called when a worker PTY exits unexpectedly. Unlike `shutdown_worker`,
    /// this does not try to remove from mux (it's already gone) and preserves the
    /// worktree directory for potential respawn.
    pub fn mark_worker_crashed(&mut self, name: &str) {
        // Remove from worker tracking
        self.worker_names.retain(|n| n != name);

        // Update event detector (suppresses future events from this worker)
        self.event_detector.remove_worker(name);

        // Update pane grid for navigation
        self.pane_grid = PaneGrid::new(&self.worker_names, &self.supervisor_name, self.is_tabbed);

        // Ensure selected tab is still valid
        self.clamp_selected_worker_tab();

        // Note: We don't remove from mux here because the pane is already gone
        // We also don't clean up the worktree - it may have uncommitted work
        // and can be used for respawn

        // Sync pane sizes to adjust layout
        let _ = self.sync_pane_sizes();

        let workers_remaining = self.worker_names.len().to_string();
        crate::telemetry::track(
            "factory_worker_crashed",
            vec![("workers_remaining", &workers_remaining)],
        );
    }

    /// Respawn a crashed worker
    ///
    /// Re-creates a worker with the same name, reusing its existing worktree if available.
    pub fn respawn_worker(
        &mut self,
        name: &str,
        teams: Option<cas_mux::TeamsSpawnConfig>,
    ) -> anyhow::Result<()> {
        crate::telemetry::track(
            "factory_worker_respawn_requested",
            vec![("worktrees_enabled", bool_prop(self.worktrees_enabled()))],
        );

        // Check if worker is already active
        if self.worker_names.contains(&name.to_string()) {
            crate::telemetry::track(
                "factory_worker_respawn_result",
                vec![("success", "false"), ("reason", "already_active")],
            );
            anyhow::bail!("Worker '{name}' is already active");
        }

        // Check if worktree exists (for worktree mode, always branch from current branch)
        let (cwd, cas_root) = if let Some(manager) = &mut self.worktree_manager {
            let worktree = match manager.ensure_worker_worktree(name) {
                Ok(worktree) => worktree,
                Err(e) => {
                    crate::telemetry::track(
                        "factory_worker_respawn_result",
                        vec![("success", "false"), ("reason", "ensure_worktree_failed")],
                    );
                    return Err(e.into());
                }
            };
            (worktree.path.clone(), Some(self.cas_dir.clone()))
        } else {
            // No worktrees - use main cwd
            let cwd = std::env::current_dir()?;
            (cwd, None)
        };

        // Add pane to mux (spawns new Claude process)
        if let Err(e) = self.mux.add_worker(
            name,
            cwd,
            cas_root.as_ref(),
            &self.supervisor_name,
            teams.as_ref(),
        ) {
            crate::telemetry::track(
                "factory_worker_respawn_result",
                vec![("success", "false"), ("reason", "mux_add_worker_failed")],
            );
            return Err(e.into());
        }

        // Track the worker name
        self.worker_names.push(name.to_string());
        crate::ui::factory::app::queue_codex_worker_intro_prompt(
            self.cas_dir(),
            name,
            self.worker_cli,
        );

        // Update pane grid for navigation
        self.pane_grid = PaneGrid::new(&self.worker_names, &self.supervisor_name, self.is_tabbed);

        // Sync pane sizes
        let _ = self.sync_pane_sizes();

        let workers_active = self.worker_names.len().to_string();
        crate::telemetry::track(
            "factory_worker_respawn_result",
            vec![("success", "true"), ("workers_active", &workers_active)],
        );

        Ok(())
    }

    /// Shutdown N workers (least recently used first, or by name)
    ///
    /// If count is 0 or None, shuts down all workers.
    ///
    /// # Arguments
    /// * `count` - Number of workers to shutdown (0 or None = all)
    /// * `names` - Specific worker names to shutdown (overrides count)
    /// * `force` - Reserved for compatibility; supervisor should pre-check worktree safety
    pub fn shutdown_workers(
        &mut self,
        count: Option<usize>,
        names: &[String],
        force: bool,
    ) -> anyhow::Result<usize> {
        let scope = shutdown_scope(count, names);
        let requested = if !names.is_empty() {
            names.len()
        } else {
            count.unwrap_or(0)
        };
        let requested_count = requested.to_string();
        crate::telemetry::track(
            "factory_worker_shutdown_requested",
            vec![
                ("scope", scope),
                ("requested_count", &requested_count),
                ("force", bool_prop(force)),
            ],
        );

        let mut shutdown_count = 0;
        let mut failures = Vec::new();

        if !names.is_empty() {
            // Shutdown specific workers by name
            for name in names {
                if let Err(e) = self.shutdown_worker(name, force) {
                    failures.push(format!("{name}: {e}"));
                } else {
                    shutdown_count += 1;
                }
            }
        } else {
            // Shutdown by count (0 = all)
            let target = count.unwrap_or(0);
            let workers_to_shutdown: Vec<String> = if target == 0 {
                self.worker_names.clone()
            } else {
                self.worker_names.iter().take(target).cloned().collect()
            };

            for name in workers_to_shutdown {
                if let Err(e) = self.shutdown_worker(&name, force) {
                    failures.push(format!("{name}: {e}"));
                } else {
                    shutdown_count += 1;
                }
            }
        }

        if !failures.is_empty() {
            let summary = failures.join("; ");
            self.set_error(format!("Shutdown had failures: {summary}"));
            let shutdown_count_str = shutdown_count.to_string();
            let failure_count_str = failures.len().to_string();
            crate::telemetry::track(
                "factory_worker_shutdown_result",
                vec![
                    ("success", "false"),
                    ("scope", scope),
                    ("shutdown_count", &shutdown_count_str),
                    ("failure_count", &failure_count_str),
                ],
            );
            anyhow::bail!("Shutdown had failures: {summary}");
        }

        let shutdown_count_str = shutdown_count.to_string();
        crate::telemetry::track(
            "factory_worker_shutdown_result",
            vec![
                ("success", "true"),
                ("scope", scope),
                ("shutdown_count", &shutdown_count_str),
            ],
        );

        Ok(shutdown_count)
    }
}
