use crate::hooks::handlers::*;

pub fn handle_session_start(
    input: &HookInput,
    cas_root: Option<&Path>,
) -> Result<HookOutput, MemError> {
    let timer = TraceTimer::new();

    // Record session start for analytics and register agent
    if let Some(cas_root) = cas_root {
        let mut stores = HookStores::new(cas_root);

        if let Some(sqlite_store) = stores.sqlite() {
            let session = Session::new(
                input.session_id.clone(),
                input.cwd.clone(),
                input.permission_mode.clone(),
            );
            if sqlite_store.start_session(&session).is_ok() {
                eprintln!(
                    "cas: Session {} started",
                    &input.session_id[..8.min(input.session_id.len())]
                );
            }
        }

        // Notify daemon via socket for instant agent registration
        // Daemon tracks PID → session mapping in memory (no files needed)
        // Pass agent_name and agent_role from this process's env (set by factory mode)
        use crate::agent_id::get_cc_pid_for_hook;
        let cc_pid = get_cc_pid_for_hook();
        let agent_name = std::env::var("CAS_AGENT_NAME").ok();
        let agent_role = std::env::var("CAS_AGENT_ROLE").ok();
        let clone_path = std::env::var("CAS_CLONE_PATH").ok();

        // Helper to register agent directly in database
        let register_directly = |stores: &mut HookStores| {
            if let Some(agent_store) = stores.agents() {
                use crate::orchestration::names as friendly_names;
                use crate::types::{Agent, AgentRole};

                let name = agent_name.clone().unwrap_or_else(friendly_names::generate);
                let mut agent = Agent::new(input.session_id.clone(), name);
                agent.pid = Some(cc_pid);
                agent.machine_id = Some(Agent::get_or_generate_machine_id());

                // Set role from environment
                if let Some(ref role_str) = agent_role {
                    if let Ok(role) = role_str.parse::<AgentRole>() {
                        agent.role = role;
                    }
                }

                // Store clone path in metadata for factory workers
                if let Some(ref path) = clone_path {
                    agent
                        .metadata
                        .insert("clone_path".to_string(), path.clone());
                }

                if let Err(reg_err) = agent_store.register(&agent) {
                    eprintln!("cas: Failed to register agent: {reg_err}");
                } else {
                    eprintln!(
                        "cas: Registered agent directly (pid: {cc_pid}, role: {agent_role:?})"
                    );
                }
            }
        };

        #[cfg(feature = "mcp-server")]
        {
            use crate::mcp::socket::{DaemonEvent, send_event};
            let event = DaemonEvent::SessionStart {
                session_id: input.session_id.clone(),
                agent_name: agent_name.clone(),
                agent_role: agent_role.clone(),
                cc_pid,
                clone_path: clone_path.clone(),
            };
            match send_event(cas_root, &event) {
                Ok(_) => eprintln!(
                    "cas: Notified daemon of session start (pid: {}, role: {:?})",
                    cc_pid,
                    std::env::var("CAS_AGENT_ROLE").ok()
                ),
                Err(e) => {
                    // Daemon socket not available - register directly in database as fallback
                    eprintln!("cas: Daemon not available ({e}), registering directly");
                    register_directly(&mut stores);
                }
            }
        }

        #[cfg(not(feature = "mcp-server"))]
        {
            // Without MCP server, register directly
            register_directly(&mut stores);
        }

        // Write OTEL context for telemetry correlation
        let project_id = crate::cloud::get_project_canonical_id();
        let project_path = cas_root.parent().map(|p| p.to_string_lossy().to_string());

        // Check for active task (reuses cached task store)
        let active_task_id = stores
            .tasks()
            .and_then(|ts| {
                ts.list(Some(TaskStatus::InProgress))
                    .ok()
                    .and_then(|tasks| tasks.first().map(|t| t.id.clone()))
            });

        let otel_ctx = OtelContext::new(input.session_id.clone())
            .with_project_id(project_id)
            .with_project_path(project_path)
            .with_permission_mode(input.permission_mode.clone())
            .with_task_id(active_task_id);

        if let Err(e) = otel_ctx.write(cas_root) {
            eprintln!("cas: Warning: Failed to write OTEL context: {e}");
        }

        // Cleanup orphaned tasks from crashed/interrupted previous sessions
        let reopened = cleanup_orphaned_tasks(cas_root);
        if reopened > 0 {
            eprintln!("cas: Reopened {reopened} orphaned task(s) from previous session");
        }
    }

    // Check if we're in plan mode
    let is_plan_mode = input.permission_mode.as_deref() == Some("plan");

    // Load config to check AI context setting
    let config = cas_root
        .map(|r| Config::load(r).unwrap_or_default())
        .unwrap_or_default();

    // Need cas_root for context building
    let cas_root = match cas_root {
        Some(root) => root,
        None => return Ok(HookOutput::empty()),
    };

    // Build appropriate context based on mode
    let context = if is_plan_mode {
        eprintln!("cas: Plan mode detected, building planning context");
        build_plan_context(input, 10, cas_root)?
    } else if config.hooks.as_ref().map(|h| h.ai_context).unwrap_or(false) {
        // Try AI-powered context selection
        eprintln!("cas: Using AI-assisted context prioritization");
        match build_context_ai(input, 5, cas_root) {
            Ok(ctx) => ctx,
            Err(e) => {
                // Check if fallback is enabled
                let ai_fallback = config.hooks.as_ref().map(|h| h.ai_fallback).unwrap_or(true);
                if ai_fallback {
                    eprintln!("cas: AI context failed ({e}), falling back to standard");
                    build_context(input, 5, cas_root)?
                } else {
                    eprintln!("cas: AI context failed: {e}");
                    return Err(e);
                }
            }
        }
    } else {
        build_context(input, 5, cas_root)?
    };

    let output = if context.is_empty() {
        HookOutput::empty()
    } else {
        HookOutput::with_context("SessionStart", context.clone())
    };

    // Record trace if dev mode is enabled
    if let Some(tracer) = DevTracer::get() {
        if tracer.should_trace_hooks() {
            let input_json = serde_json::json!({
                "session_id": input.session_id,
                "cwd": input.cwd,
                "permission_mode": input.permission_mode,
            });
            let output_json = serde_json::json!({
                "has_context": !context.is_empty(),
                "context_length": context.len(),
            });

            let _ = tracer.record_hook(
                "SessionStart",
                &input_json,
                &output_json,
                if context.is_empty() {
                    None
                } else {
                    Some(&context)
                },
                Some(estimate_tokens(&context)),
                timer.elapsed_ms(),
                true,
                None,
            );
        }
    }

    Ok(output)
}

/// Estimate token count (rough approximation: ~4 chars per token)
pub(crate) fn estimate_tokens(s: &str) -> usize {
    s.len() / 4
}

/// Compute session outcome based on metrics and friction events
///
/// Outcome determination priority:
/// Handle SessionEnd hook - generate session summary and mark for extraction
pub fn handle_session_end(
    input: &HookInput,
    cas_root: Option<&Path>,
) -> Result<HookOutput, MemError> {
    let cas_root = match cas_root {
        Some(root) => root,
        None => return Ok(HookOutput::empty()),
    };

    let mut stores = HookStores::new(cas_root);

    // Get observations from this session
    let entry_store = stores.entries()?;
    let entries = entry_store.list()?;
    let session_observations: Vec<_> = entries
        .iter()
        .filter(|e| e.session_id.as_deref() == Some(&input.session_id))
        .collect();

    let session_count = session_observations.len();

    // Clean up agent leases and reset task status - ALWAYS do this regardless of observation count
    cleanup_agent_leases(cas_root, &input.session_id);

    // Notify daemon via socket that session ended
    #[cfg(feature = "mcp-server")]
    {
        use crate::agent_id::get_cc_pid_for_hook;
        use crate::mcp::socket::{DaemonEvent, send_event};
        let cc_pid = get_cc_pid_for_hook();
        let event = DaemonEvent::SessionEnd {
            session_id: input.session_id.clone(),
            cc_pid: Some(cc_pid),
        };
        if send_event(cas_root, &event).is_ok() {
            eprintln!("cas: Notified daemon of session end");
        }
    }

    // Clean up current_session file
    let _ = std::fs::remove_file(cas_root.join("current_session"));

    // Clean up session files used for context boosting
    clear_session_files(cas_root);

    // Clean up OTEL context file
    let _ = OtelContext::remove(cas_root);

    // Clean up verifier marker file (safety cleanup in case subagent didn't clean up)
    let _ = std::fs::remove_file(cas_root.join(".verifier_unjail_marker"));

    if session_count == 0 {
        eprintln!(
            "cas: Session {} ended (no observations)",
            &input.session_id[..8.min(input.session_id.len())]
        );
        return Ok(HookOutput::empty());
    }

    // Log session end
    eprintln!(
        "cas: Session {} ended with {} observations",
        &input.session_id[..8.min(input.session_id.len())],
        session_count
    );

    // Check if AI features are enabled
    let config = Config::load(cas_root).unwrap_or_default();
    let should_summarize = config
        .hooks
        .as_ref()
        .map(|h| h.generate_summaries)
        .unwrap_or(false);

    // Generate session title and compute outcome (reuses single SqliteStore)
    if let Some(sqlite_store) = stores.sqlite() {
        match generate_session_title_sync(&session_observations) {
            Ok(title) => {
                if sqlite_store
                    .update_session_title(&input.session_id, &title)
                    .is_ok()
                {
                    eprintln!("cas: Session title: {title}");
                }
            }
            Err(e) => {
                eprintln!("cas: Title generation failed: {e}");
            }
        }

        // Compute session outcome
        let session_opt = sqlite_store.get_session(&input.session_id).ok().flatten();

        let outcome = if let Some(session) = session_opt {
            if session.tasks_closed > 0 {
                cas_types::SessionOutcome::TasksCompleted
            } else if session.entries_created > 0 {
                cas_types::SessionOutcome::LearningsCreated
            } else if session.tool_uses > 0 {
                cas_types::SessionOutcome::Exploration
            } else {
                cas_types::SessionOutcome::Abandoned
            }
        } else if session_count > 0 {
            cas_types::SessionOutcome::Exploration
        } else {
            cas_types::SessionOutcome::Abandoned
        };

        if sqlite_store
            .update_session_signals(&input.session_id, Some(outcome), None, None)
            .is_ok()
        {
            eprintln!("cas: Session outcome: {outcome}");
        }
    }

    if should_summarize {
        // Generate summary
        let entry_store = stores.entries()?;
        {
            if let Ok(summary) = generate_session_summary_sync(&session_observations) {
                // Store the summary as a context entry
                if !summary.summary.is_empty() {
                    let id = entry_store.generate_id()?;
                    let mut content = format!("## Session Summary\n\n{}\n", summary.summary);

                    if !summary.decisions.is_empty() {
                        content.push_str("\n### Decisions\n");
                        for decision in &summary.decisions {
                            content.push_str(&format!("- {decision}\n"));
                        }
                    }

                    if !summary.key_learnings.is_empty() {
                        content.push_str("\n### Learnings\n");
                        for learning in &summary.key_learnings {
                            content.push_str(&format!("- {learning}\n"));
                        }
                    }

                    if !summary.follow_up_tasks.is_empty() {
                        content.push_str("\n### Follow-up Tasks\n");
                        for task in &summary.follow_up_tasks {
                            content.push_str(&format!("- {task}\n"));
                        }
                    }

                    let entry = Entry {
                        id: id.clone(),
                        entry_type: EntryType::Context,
                        content,
                        tags: vec!["session-summary".to_string()],
                        session_id: Some(input.session_id.clone()),
                        ..Default::default()
                    };

                    if entry_store.add(&entry).is_ok() {
                        eprintln!("cas: Generated session summary: {id}");
                    }
                }
            }
        }
    }

    Ok(HookOutput::empty())
}

/// Generate session summary using AI (synchronous wrapper with timeout)
pub(crate) fn generate_session_summary_sync(
    observations: &[&Entry],
) -> Result<SessionSummary, MemError> {
    use std::time::Duration;
    use tokio::runtime::Runtime;

    let rt =
        Runtime::new().map_err(|e| MemError::Other(format!("Failed to create runtime: {e}")))?;

    // 5 second timeout to prevent blocking the hook for too long
    rt.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(5),
            generate_session_summary_async(observations),
        )
        .await
        .map_err(|_| MemError::Other("AI summary generation timed out after 5s".to_string()))?
    })
}

/// Generate session summary using AI
async fn generate_session_summary_async(
    observations: &[&Entry],
) -> Result<SessionSummary, MemError> {
    use crate::tracing::claude_wrapper::traced_prompt;
    use claude_rs::QueryOptions;

    // Build prompt from observations
    let obs_text: String = observations
        .iter()
        .take(50) // Limit to prevent token overflow
        .map(|e| {
            format!(
                "- [{}] {}",
                e.source_tool.as_deref().unwrap_or("?"),
                e.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt_text = format!(
        r#"Analyze these observations from a coding session and generate a structured summary.

## Observations
{obs_text}

## Task
Generate a JSON summary with:
- summary: 1-2 sentence overview of what was accomplished
- decisions: Array of key decisions made (architectural, design, etc.)
- tasks_completed: Array of tasks that were finished
- key_learnings: Array of important discoveries or patterns learned
- follow_up_tasks: Array of suggested next tasks

Respond with JSON only, no markdown:
{{"summary": "...", "decisions": [...], "tasks_completed": [...], "key_learnings": [...], "follow_up_tasks": [...]}}"#
    );

    let result = traced_prompt(
        &prompt_text,
        QueryOptions::new().model("claude-haiku-4-5").max_turns(1),
        "session_summary",
    )
    .await
    .map_err(|e| MemError::Other(format!("AI summary failed: {e}")))?;

    let response_text = result.text();

    // Parse JSON response
    let json_str = response_text
        .find('{')
        .and_then(|start| {
            response_text
                .rfind('}')
                .map(|end| &response_text[start..=end])
        })
        .unwrap_or(response_text);

    serde_json::from_str(json_str)
        .map_err(|e| MemError::Parse(format!("Failed to parse summary: {e}")))
}

/// Generate session title (synchronous wrapper with timeout)
pub fn generate_session_title_sync(observations: &[&Entry]) -> Result<String, MemError> {
    use std::time::Duration;
    use tokio::runtime::Runtime;

    let rt =
        Runtime::new().map_err(|e| MemError::Other(format!("Failed to create runtime: {e}")))?;

    // 15 second timeout - claude CLI spawn can take a few seconds
    rt.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(15),
            generate_session_title_async(observations),
        )
        .await
        .map_err(|_| MemError::Other("Title generation timed out after 15s".to_string()))?
    })
}

/// Generate a concise session title using AI
async fn generate_session_title_async(observations: &[&Entry]) -> Result<String, MemError> {
    use crate::tracing::claude_wrapper::traced_prompt;
    use claude_rs::QueryOptions;

    if observations.is_empty() {
        return Ok("Empty session".to_string());
    }

    // Build a brief summary of what happened
    let obs_text: String = observations
        .iter()
        .take(20) // Limit to key observations
        .map(|e| {
            let tool = e.source_tool.as_deref().unwrap_or("?");
            let content = truncate_display(&e.content, 100);
            format!("- [{tool}] {content}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt_text = format!(
        r#"Generate a 5-8 word title summarizing this coding session.

## Session Activity
{obs_text}

## Examples of good titles:
- "Implemented user authentication flow"
- "Fixed payment processing bug"
- "Refactored database queries for performance"
- "Added dark mode support"
- "Set up CI/CD pipeline"

Respond with ONLY the title, no quotes or punctuation at the end."#
    );

    let result = traced_prompt(
        &prompt_text,
        QueryOptions::new().model("claude-haiku-4-5").max_turns(1),
        "session_title",
    )
    .await
    .map_err(|e| MemError::Other(format!("Title generation failed: {e}")))?;

    let title = result.text().trim().to_string();

    // Clean up the title - remove quotes if present
    let title = title.trim_matches('"').trim_matches('\'').to_string();

    // Ensure reasonable length
    if title.chars().count() > 100 {
        Ok(title.chars().take(100).collect())
    } else if title.is_empty() {
        Ok("Coding session".to_string())
    } else {
        Ok(title)
    }
}

/// Extract learnings from transcript (synchronous wrapper with timeout)
pub(crate) fn extract_learnings_sync(
    transcript_path: &str,
    file_paths: &[String],
) -> Result<Vec<ExtractedLearning>, MemError> {
    use std::time::Duration;
    use tokio::runtime::Runtime;

    let rt =
        Runtime::new().map_err(|e| MemError::Other(format!("Failed to create runtime: {e}")))?;

    // 5 second timeout to prevent blocking the hook for too long
    rt.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(5),
            extract_learnings_async(transcript_path, file_paths),
        )
        .await
        .map_err(|_| MemError::Other("Learning extraction timed out after 5s".to_string()))?
    })
}

/// Extract learnings from transcript using AI
///
/// Reads the transcript, sends to Haiku to identify project conventions
/// that the user taught Claude during the session.
async fn extract_learnings_async(
    transcript_path: &str,
    file_paths: &[String],
) -> Result<Vec<ExtractedLearning>, MemError> {
    use crate::tracing::claude_wrapper::traced_prompt;
    use claude_rs::QueryOptions;

    // Read the transcript file
    let transcript = std::fs::read_to_string(transcript_path)
        .map_err(|e| MemError::Other(format!("Failed to read transcript: {e}")))?;

    // Skip if transcript is too short (likely no meaningful interaction)
    if transcript.len() < 500 {
        return Ok(vec![]);
    }

    // Truncate transcript if too long (keep last 50k chars - most recent context)
    // Find a valid char boundary to avoid slicing in the middle of multi-byte UTF-8 chars
    let transcript_excerpt = if transcript.len() > 50000 {
        let mut start = transcript.len() - 50000;
        // Walk forward to find a valid UTF-8 char boundary
        while start < transcript.len() && !transcript.is_char_boundary(start) {
            start += 1;
        }
        &transcript[start..]
    } else {
        &transcript
    };

    // Build file context from observed paths
    let file_context = if file_paths.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n## Files Modified This Session\n{}",
            file_paths
                .iter()
                .take(20)
                .map(|p| format!("- {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let prompt_text = format!(
        r#"Analyze this Claude Code session transcript and extract project-specific rules or conventions that the USER TAUGHT Claude.

## What to Look For
- User corrections: "No, don't do X, instead do Y"
- User preferences: "Always use X pattern", "Never import from Y"
- API corrections: "That function doesn't exist, use Z instead"
- Framework conventions: "In this project we use X for Y"
- Style rules: "We don't use useEffect here", "Always use generated types"

## What to IGNORE
- General programming knowledge (not project-specific)
- Claude's own discoveries without user confirmation
- One-off fixes that aren't conventions
- Debugging steps

## Transcript
{transcript_excerpt}
{file_context}

## Task
Extract 0-5 project-specific rules the user taught. For each, include:
- content: The rule in imperative form ("Use X", "Never Y", "Always Z")
- path_pattern: Glob pattern for files this applies to (e.g., "**/*.tsx", "lib/**/*.ex") or null if global
- confidence: 0.7-1.0 based on how explicit the user was
- tags: Relevant tags like ["react", "elixir", "testing"]

Respond with JSON array only, no markdown:
[{{"content": "...", "path_pattern": "...", "confidence": 0.9, "tags": ["..."]}}]

If no clear learnings found, respond with: []"#
    );

    let result = traced_prompt(
        &prompt_text,
        QueryOptions::new().model("claude-haiku-4-5").max_turns(1),
        "learning_extraction",
    )
    .await
    .map_err(|e| MemError::Other(format!("Learning extraction failed: {e}")))?;

    let response_text = result.text();

    // Parse JSON response
    let json_str = response_text
        .find('[')
        .and_then(|start| {
            response_text
                .rfind(']')
                .map(|end| &response_text[start..=end])
        })
        .unwrap_or("[]");

    let learnings: Vec<ExtractedLearning> = serde_json::from_str(json_str)
        .map_err(|e| MemError::Parse(format!("Failed to parse learnings: {e}")))?;

    // Filter out low-confidence learnings
    Ok(learnings
        .into_iter()
        .filter(|l| l.confidence >= 0.7)
        .collect())
}
