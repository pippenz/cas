use std::sync::Arc;

use anyhow::Context;

use crate::mcp::server::CasCore;
use crate::store::open_agent_store;

/// Run the MCP server with 13 meta-tools (11 CAS + 2 proxy)
pub async fn run_server() -> anyhow::Result<()> {
    run_server_impl().await
}

/// Internal implementation for running the MCP server
async fn run_server_impl() -> anyhow::Result<()> {
    let enable_daemon = true;
    use crate::cloud::{CloudConfig, CloudSyncer, CloudSyncerConfig, SyncQueue};
    use crate::mcp::daemon::{EmbeddedDaemonConfig, spawn_daemon};
    use crate::mcp::tools::CasService;
    use crate::store::find_cas_root;
    use crate::store::{open_rule_store, open_skill_store, open_store, open_task_store};
    use rmcp::ServiceExt;
    use rmcp::transport::stdio;

    let cas_root = find_cas_root().map_err(|_| {
        anyhow::anyhow!("CAS not initialized. Run `cas init` in your project first.")
    })?;

    // Install panic hook before anything else can panic. Routes panics to a
    // dedicated file under `.cas/logs/cas-serve-{date}.log` with timestamp,
    // PID, and a full backtrace. The default hook still runs so stderr goes
    // to the MCP client as before.
    //
    // Without this, panics in tool handlers kill the process and the crash
    // output is lost — the MCP client only sees "Connection closed" and the
    // auto-respawn path gives us no diagnostic trail.
    install_serve_panic_hook(&cas_root);

    // Register this repo in the host-scoped known_repos registry. Fires
    // every time `cas serve` starts in a directory with `.cas/`, catching
    // repos that pre-date the `cas init` registration hook. Non-fatal:
    // failure here must not block MCP serve startup.
    if let Some(repo_root) = cas_root.parent() {
        crate::store::known_repos::register_repo(repo_root);
    }

    // Opportunistic cross-repo sweep — debounced via
    // `~/.cas/last_global_sweep`. Runs on a detached blocking task so MCP
    // startup is NEVER delayed. Any panic is caught and logged; any error
    // is warn-logged. This is Unit 3's keystone wiring (EPIC cas-7c88).
    let sweep_cas_config = crate::config::Config::load(&cas_root).unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let wt_cfg = sweep_cas_config.worktrees().clone();
        match crate::worktree::sweep::opportunistic::run_if_due(&wt_cfg) {
            Ok(Some(summary)) => {
                eprintln!(
                    "[CAS] opportunistic sweep: visited {} repo(s), reclaimed {}, salvaged {}",
                    summary.repos_visited, summary.reclaimed, summary.salvaged,
                );
            }
            Ok(None) => {
                // Skipped by debounce — no user-visible output.
            }
            Err(e) => {
                tracing::error!(error = %e, "opportunistic sweep failed");
            }
        }
    });

    // Run startup cloud pull in a background task with a short timeout
    // so a slow/unreachable cloud endpoint never blocks MCP server startup.
    //
    // Hold the JoinHandle so that if `eager_init_stores` aborts startup we can
    // cancel this task instead of leaving it racing the dying process to open
    // the same DB that just refused to open (cas-5c05 review A2).
    let cloud_sync_handle = {
        let cas_root_bg = cas_root.clone();
        tokio::task::spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tokio::task::spawn_blocking(move || {
                    let cloud_config = match CloudConfig::load_from_cas_dir(&cas_root_bg) {
                        Ok(c) if c.is_logged_in() => c,
                        _ => return,
                    };
                    let queue = match SyncQueue::open(&cas_root_bg) {
                        Ok(q) => {
                            let _ = q.init();
                            q
                        }
                        Err(_) => return,
                    };
                    let config = CloudSyncerConfig {
                        timeout: std::time::Duration::from_secs(5),
                        ..Default::default()
                    };
                    let syncer = CloudSyncer::new(std::sync::Arc::new(queue), cloud_config, config);
                    let Ok(store) = open_store(&cas_root_bg) else {
                        return;
                    };
                    let Ok(task_store) = open_task_store(&cas_root_bg) else {
                        return;
                    };
                    let Ok(rule_store) = open_rule_store(&cas_root_bg) else {
                        return;
                    };
                    let Ok(skill_store) = open_skill_store(&cas_root_bg) else {
                        return;
                    };

                    match syncer.pull(
                        store.as_ref(),
                        task_store.as_ref(),
                        rule_store.as_ref(),
                        skill_store.as_ref(),
                    ) {
                        Ok(result) if result.total_pulled() > 0 => {
                            eprintln!("[CAS] Synced {} items from cloud", result.total_pulled());
                        }
                        Err(e) => {
                            eprintln!("[CAS] Cloud sync failed (continuing): {e}");
                        }
                        _ => {}
                    }
                }),
            )
            .await;
            if result.is_err() {
                eprintln!("[CAS] Cloud sync timed out (continuing without sync)");
            }
        })
    };

    let (daemon, activity, _handle) = if enable_daemon {
        let cas_config = crate::config::Config::load(&cas_root).unwrap_or_default();
        let code_config = cas_config.code();
        let project_dir = cas_root.parent().unwrap_or(&cas_root);
        let code_watch_paths: Vec<std::path::PathBuf> = code_config
            .watch_paths
            .iter()
            .map(|p| project_dir.join(p))
            .collect();

        let config = EmbeddedDaemonConfig {
            cas_root: cas_root.clone(),
            index_code: code_config.enabled,
            code_watch_paths,
            code_extensions: code_config.extensions.clone(),
            code_exclude_patterns: code_config.exclude_patterns.clone(),
            code_index_interval_secs: code_config.index_interval_secs,
            code_debounce_ms: code_config.debounce_ms,
            ..Default::default()
        };
        let (daemon, handle) = spawn_daemon(config);
        let activity = daemon.activity_tracker();
        (Some(daemon), Some(activity), Some(handle))
    } else {
        (None, None, None)
    };

    let core = CasCore::with_daemon(cas_root.clone(), activity, daemon.clone());

    // Eagerly initialize all stores before serving MCP requests.
    // This moves cold-start overhead (connection open, schema init) out of the
    // first tool call path, preventing timeouts on the initial request.
    //
    // Failure here is fatal: a partially-initialized server would respond to
    // `tools/list` with the full registry but every call would error, which is
    // the silent-degradation mode this guard exists to prevent (cas-5c05).
    if let Err(e) = eager_init_stores(&core, &cas_root) {
        // Cancel the cloud-sync task before bubbling the error so it stops
        // racing for the same DB during the parent's shutdown window.
        cloud_sync_handle.abort();
        return Err(e);
    }

    // Eager auto-registration for factory workers where SessionStart hook may not fire.
    // When CAS_SESSION_ID is set (by PtyConfig::claude()), register immediately so the
    // agent appears in worker_status before any MCP tool call is made.
    if let Ok(session_id) = std::env::var("CAS_SESSION_ID") {
        if !session_id.is_empty() {
            let agent_name =
                std::env::var("CAS_AGENT_NAME").unwrap_or_else(|_| "worker".to_string());
            eprintln!(
                "[CAS] Eager registration: {} ({})",
                agent_name,
                &session_id[..8.min(session_id.len())]
            );
            match core.register_agent(session_id.clone(), agent_name, None) {
                Ok(_) => {
                    // Tell the daemon so it sends heartbeats
                    if let Some(ref d) = daemon {
                        let d = Arc::clone(d);
                        let sid = session_id.clone();
                        tokio::spawn(async move {
                            d.set_agent_id(sid).await;
                        });
                    }
                }
                Err(e) => {
                    eprintln!("[CAS] Eager registration failed: {e}");
                }
            }
        }
    }

    // Load MCP proxy config from .cas/proxy.toml (project) and ~/.config/code-mode-mcp/config.toml (user)
    #[cfg(feature = "mcp-proxy")]
    let proxy = {
        let proxy_path = cas_root.join("proxy.toml");
        let cfg = cmcp_core::config::Config::load_merged(if proxy_path.exists() {
            Some(&proxy_path)
        } else {
            None
        });
        match cfg {
            Ok(cfg) if !cfg.servers.is_empty() => {
                eprintln!(
                    "[CAS] Connecting to {} upstream MCP server(s)...",
                    cfg.servers.len()
                );
                match cmcp_core::ProxyEngine::from_configs(cfg.servers).await {
                    Ok(engine) => {
                        let count = engine.tool_count().await;
                        eprintln!("[CAS] MCP proxy ready ({count} upstream tools)");
                        write_proxy_catalog_cache(&cas_root, &engine).await;
                        Some(std::sync::Arc::new(engine))
                    }
                    Err(e) => {
                        eprintln!("[CAS] MCP proxy init failed (continuing without proxy): {e}");
                        None
                    }
                }
            }
            _ => None,
        }
    };
    #[cfg(not(feature = "mcp-proxy"))]
    let _proxy: Option<()> = None;

    // Register proxy with daemon for hot-reload watching
    #[cfg(feature = "mcp-proxy")]
    if let (Some(d), Some(p)) = (&daemon, &proxy) {
        d.set_proxy(Arc::clone(p)).await;
    }

    #[cfg(feature = "mcp-proxy")]
    let proxy_active = proxy.is_some();
    #[cfg(not(feature = "mcp-proxy"))]
    let proxy_active = false;

    #[cfg(feature = "mcp-proxy")]
    let service = CasService::new(core, proxy);
    #[cfg(not(feature = "mcp-proxy"))]
    let service = CasService::new(core);

    // Empty-registry guard — if the tool router somehow ends up empty, refuse
    // to start. Otherwise the server would respond to `tools/list` with `[]`
    // and the MCP client (e.g. Claude Code) silently shows zero CAS tools to
    // the agent with no surfaced error. See cas-5c05.
    let tool_names = service.registered_tool_names();
    if tool_names.is_empty() {
        anyhow::bail!(
            "MCP tool registry is empty. This is a CAS build bug — refusing to \
             start a server that would silently expose zero tools to the client. \
             Rebuild CAS and retry."
        );
    }
    eprintln!(
        "[CAS] Starting MCP server ({} tools: {}{})",
        tool_names.len(),
        tool_names.join(", "),
        if proxy_active { ", proxy active" } else { "" }
    );

    let server = service.serve(stdio()).await?;
    if let Err(e) = server.waiting().await {
        eprintln!("[CAS] MCP server terminated with error: {e}");
    }

    eprintln!("[CAS] Shutting down, releasing tasks...");
    {
        use crate::agent_id::read_session_for_mcp;
        if let Ok(agent_id) = read_session_for_mcp(&cas_root) {
            if let Err(e) = release_agent_tasks(&cas_root, &agent_id) {
                eprintln!("[CAS] Failed to release agent tasks for {agent_id}: {e}");
            }
        }
    }

    if let Some(d) = daemon {
        d.shutdown();
    }

    Ok(())
}

/// Total time budget for the eager store-init phase before `cas serve` aborts.
///
/// This budget exists to convert silent zero-tools mode into a loud failure —
/// not to time-police healthy startup. Three forces set its value:
///
/// 1. **Real-incident floor.** The cas-5c05 trigger was a 15-hour `cas init`
///    hang on the same project. Anything in the seconds-to-minute range
///    catches that with massive headroom.
/// 2. **Thundering-herd ceiling.** investigation-mcp-worktree.md (cas-09f1,
///    2026-03-25) documents 6 concurrent `cas serve` processes opening the
///    same `cas.db`. Each store has a 5s SQLite `busy_timeout`, so realistic
///    cross-process contention can stack to a low-tens-of-seconds for a
///    legitimate factory startup. The budget must tolerate that.
/// 3. **MCP client deadline.** Claude Code's `initialize`/`tools/list`
///    handshake gives up around 60s. The budget must be strictly less so the
///    abort surfaces as a visible error to the client rather than racing the
///    client's own timeout.
///
/// 45s sits comfortably between all three: ~1200× the realistic contention
/// floor, ~15s margin under the MCP client deadline, and orders of magnitude
/// shorter than any pathological hang the original incident exhibited.
/// Tuned per cas-5c05 review (supervisor verification).
const EAGER_INIT_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);

/// Eagerly open every store and the search index before serving MCP requests.
///
/// Returns an error (which `cas serve` propagates as a non-zero exit) if any
/// store fails to open or if the total init phase exceeds `EAGER_INIT_BUDGET`.
/// This converts the previously silent failure mode (server starts, registry
/// looks fine to the client, but every tool call later errors) into a loud
/// startup failure that the parent factory can detect and report.
fn eager_init_stores(
    core: &CasCore,
    cas_root: &std::path::Path,
) -> anyhow::Result<()> {
    let start = std::time::Instant::now();

    let step = |name: &'static str,
                f: &mut dyn FnMut() -> Result<(), anyhow::Error>|
     -> anyhow::Result<()> {
        if start.elapsed() > EAGER_INIT_BUDGET {
            anyhow::bail!(
                "store init exceeded {}s budget before reaching '{name}'. \
                 Likely cause: another process holds a write lock on \
                 {db}. Inspect with `lsof {db}` or `fuser {db}` and stop \
                 the offending process before retrying `cas serve`.",
                EAGER_INIT_BUDGET.as_secs(),
                db = cas_root.join("cas.db").display()
            );
        }
        f().with_context(|| format!("eager store init failed at '{name}'"))?;
        Ok(())
    };

    step("entry_store", &mut || {
        core.open_store().map(|_| ()).map_err(map_mcp_err)
    })?;
    step("task_store", &mut || {
        core.open_task_store().map(|_| ()).map_err(map_mcp_err)
    })?;
    step("rule_store", &mut || {
        core.open_rule_store().map(|_| ()).map_err(map_mcp_err)
    })?;
    step("skill_store", &mut || {
        core.open_skill_store().map(|_| ()).map_err(map_mcp_err)
    })?;
    step("agent_store", &mut || {
        core.open_agent_store().map(|_| ()).map_err(map_mcp_err)
    })?;
    step("entity_store", &mut || {
        core.open_entity_store().map(|_| ()).map_err(map_mcp_err)
    })?;
    step("verification_store", &mut || {
        core.open_verification_store().map(|_| ()).map_err(map_mcp_err)
    })?;
    step("worktree_store", &mut || {
        core.open_worktree_store().map(|_| ()).map_err(map_mcp_err)
    })?;
    step("search_index", &mut || {
        core.open_search_index().map(|_| ()).map_err(map_mcp_err)
    })?;
    // Note: `core.load_config()` is intentionally not in the eager-init list.
    // It returns Config (not Result) and falls back to a default on read
    // failure, so it cannot signal anything actionable to surface here. It
    // gets called lazily via the OnceLock cache on first tool dispatch.

    eprintln!(
        "[CAS] Stores initialized in {}ms",
        start.elapsed().as_millis()
    );
    Ok(())
}

fn map_mcp_err(e: rmcp::ErrorData) -> anyhow::Error {
    anyhow::anyhow!("{}", e.message)
}

/// Install a panic hook that writes panic info + backtrace to a daily log
/// under `{cas_root}/logs/cas-serve-{date}.log`.
///
/// Preserves the previous hook (so Rust's default stderr output still reaches
/// the MCP client) and appends a timestamped record to the file. Failures
/// during hook setup or write are swallowed — the hook must never itself
/// panic or abort serve startup.
fn install_serve_panic_hook(cas_root: &std::path::Path) {
    use std::io::Write;

    let log_dir = cas_root.join("logs");
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "[CAS] Warning: could not create serve log dir {}: {e}",
            log_dir.display()
        );
        return;
    }
    let today = chrono::Local::now().format("%Y-%m-%d");
    let log_path = log_dir.join(format!("cas-serve-{today}.log"));
    eprintln!("[CAS] Serve panic log: {}", log_path.display());

    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let pid = std::process::id();
            let agent = std::env::var("CAS_AGENT_NAME").unwrap_or_else(|_| "-".to_string());
            let session = std::env::var("CAS_SESSION_ID").unwrap_or_else(|_| "-".to_string());
            let _ = writeln!(
                f,
                "---\n{ts} pid={pid} agent={agent} session={session} PANIC"
            );
            let _ = writeln!(f, "{info}");
            let bt = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(f, "{bt}");
            let _ = f.flush();
        }
        default(info);
    }));
}

/// Release all tasks claimed by an agent on shutdown and unregister the agent
fn release_agent_tasks(cas_root: &std::path::Path, agent_id: &str) -> anyhow::Result<()> {
    let agent_store = open_agent_store(cas_root)?;
    agent_store.graceful_shutdown(agent_id)?;
    agent_store.clear_working_epics(agent_id)?;
    agent_store.unregister(agent_id)?;
    Ok(())
}

/// Write the proxy tool catalog to `.cas/proxy_catalog.json` for SessionStart context injection.
///
/// Writes a JSON map of `{ server_name: [tool_name, ...] }` which is consumed by
/// `build_mcp_tools_section` in hooks/context.rs.
#[cfg(feature = "mcp-proxy")]
pub async fn write_proxy_catalog_cache(
    cas_root: &std::path::Path,
    engine: &cmcp_core::ProxyEngine,
) {
    let servers = engine.catalog_entries_by_server().await;
    if servers.is_empty() {
        return;
    }
    // Convert to the format expected by build_mcp_tools_section: { server: [tool_names] }
    let simplified: std::collections::HashMap<String, Vec<String>> = servers
        .into_iter()
        .map(|(server, entries)| {
            let names = entries.into_iter().map(|e| e.name).collect();
            (server, names)
        })
        .collect();
    let cache_path = cas_root.join("proxy_catalog.json");
    match serde_json::to_string(&simplified) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&cache_path, json) {
                eprintln!("[CAS] Failed to write proxy catalog cache: {e}");
            }
        }
        Err(e) => {
            eprintln!("[CAS] Failed to serialize proxy catalog: {e}");
        }
    }
}
