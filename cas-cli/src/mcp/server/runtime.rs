use std::sync::Arc;

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

    // Run startup cloud pull in a background task with a short timeout
    // so a slow/unreachable cloud endpoint never blocks MCP server startup.
    {
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
        });
    }

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
    {
        let start = std::time::Instant::now();
        let _ = core.open_store();
        let _ = core.open_task_store();
        let _ = core.open_rule_store();
        let _ = core.open_skill_store();
        let _ = core.open_agent_store();
        let _ = core.open_entity_store();
        let _ = core.open_verification_store();
        let _ = core.open_worktree_store();
        eprintln!("[CAS] Stores initialized in {}ms", start.elapsed().as_millis());
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
    let proxy: Option<()> = None;

    // Register proxy with daemon for hot-reload watching
    #[cfg(feature = "mcp-proxy")]
    if let (Some(d), Some(p)) = (&daemon, &proxy) {
        d.set_proxy(Arc::clone(p)).await;
    }

    eprintln!(
        "[CAS] Starting MCP server (13 tools{})",
        if proxy.is_some() {
            ", proxy active"
        } else {
            ""
        }
    );
    #[cfg(feature = "mcp-proxy")]
    let service = CasService::new(core, proxy);
    #[cfg(not(feature = "mcp-proxy"))]
    let service = CasService::new(core);
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
