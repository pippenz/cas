use crate::hooks::handlers::*;

pub fn add_loop_completion_note(
    cas_root: &std::path::Path,
    task_id: &str,
    loop_state: &crate::types::Loop,
    reason: &str,
) {
    if let Ok(task_store) = crate::store::open_task_store(cas_root) {
        if let Ok(mut task) = task_store.get(task_id) {
            let note = format!(
                "\n[{}] Loop {} - {} after {} iterations",
                chrono::Utc::now().format("%Y-%m-%d %H:%M"),
                loop_state.id,
                reason,
                loop_state.iteration
            );
            task.notes.push_str(&note);
            let _ = task_store.update(&task);
        }
    }
}

/// Handle loop iteration - blocks exit and feeds prompt back
///
/// This is the core of the iteration loop functionality. When an active loop
/// exists for the session, this function:
/// 1. Checks if the completion promise was found in the transcript
/// 2. Checks if max iterations were reached
/// 3. If neither, blocks exit and injects the prompt for another iteration
pub fn handle_loop_iteration(
    input: &HookInput,
    cas_root: &std::path::Path,
    loop_store: std::sync::Arc<dyn crate::store::LoopStore>,
    active_loop: &mut crate::types::Loop,
) -> Result<HookOutput, MemError> {
    // Check for completion promise in transcript
    if let Some(ref promise) = active_loop.completion_promise {
        if let Some(ref transcript_path) = input.transcript_path {
            let path = std::path::Path::new(transcript_path);
            if check_promise_in_transcript(path, promise).unwrap_or(false) {
                // Promise found - complete the loop
                active_loop.complete(&format!("Promise '{promise}' detected"));
                let _ = loop_store.update(active_loop);
                eprintln!(
                    "cas: Loop {} completed after {} iterations (promise detected)",
                    active_loop.id, active_loop.iteration
                );

                // Add final task note if linked
                if let Some(ref task_id) = active_loop.task_id {
                    add_loop_completion_note(cas_root, task_id, active_loop, "completed");
                }

                return Ok(HookOutput::empty());
            }
        }
    }

    // Check if max iterations reached
    if active_loop.is_max_reached() {
        active_loop.max_iterations_reached();
        let _ = loop_store.update(active_loop);
        eprintln!(
            "cas: Loop {} stopped after {} iterations (max reached)",
            active_loop.id, active_loop.max_iterations
        );

        // Add final task note if linked
        if let Some(ref task_id) = active_loop.task_id {
            add_loop_completion_note(cas_root, task_id, active_loop, "max iterations reached");
        }

        return Ok(HookOutput::empty());
    }

    // Increment iteration and continue
    active_loop.increment();
    let _ = loop_store.update(active_loop);

    // Build the iteration message with honesty enforcement
    let promise_info = active_loop
        .completion_promise
        .as_ref()
        .map(|p| format!(
            "\n\n---\n⚠️ **Completion**: Output `<promise>{p}</promise>` ONLY when the task is truly complete.\n\
            The statement MUST be completely and unequivocally TRUE. Do NOT lie to exit the loop.\n\
            If the loop should stop, the promise will become true naturally when work is done.\n\
            Use `/cancel-loop` to stop early if needed."
        ))
        .unwrap_or_default();

    let iteration_msg = format!(
        "🔄 Loop iteration {}{}\n\n{}{}",
        active_loop.iteration,
        active_loop
            .max_iterations
            .checked_sub(0)
            .filter(|&m| m > 0)
            .map(|m| format!("/{m}"))
            .unwrap_or_default(),
        active_loop.prompt,
        promise_info
    );

    eprintln!(
        "cas: Loop {} iteration {} starting",
        active_loop.id, active_loop.iteration
    );

    // If linked to a task, add a progress note
    if let Some(ref task_id) = active_loop.task_id {
        if let Ok(task_store) = crate::store::open_task_store(cas_root) {
            if let Ok(mut task) = task_store.get(task_id) {
                let note = format!(
                    "\n[{}] Loop iteration {} started",
                    chrono::Utc::now().format("%Y-%m-%d %H:%M"),
                    active_loop.iteration
                );
                task.notes.push_str(&note);
                let _ = task_store.update(&task);
            }
        }
    }

    // Block exit (decision=block + reason for Claude) and surface user-visible
    // status via systemMessage. The named constructor enforces the Stop-family
    // schema invariant — hookSpecificOutput is unrepresentable here by type.
    Ok(HookOutput::block_stop_with_context(
        iteration_msg,
        format!("Loop iteration {} continuing", active_loop.iteration),
    ))
}

/// Default limit for learnings shown in review context
const LEARNING_REVIEW_LIMIT: usize = 20;

/// Build context for learning review if there are unreviewed learnings above threshold
///
/// Returns Some(context) if learning review should be triggered, None otherwise.
/// The context instructs the agent to spawn a learning-reviewer subagent.
/// Returns error context if store operations fail (fail explicitly, no silent skip).
pub fn build_learning_review_context(store: &dyn Store, config: &Config) -> Option<String> {
    // Check if learning review is enabled via hooks.stop.learning_review_enabled
    let stop_config = config.hooks.as_ref().map(|h| &h.stop);

    let enabled = stop_config
        .map(|s| s.learning_review_enabled)
        .unwrap_or(false);

    if !enabled {
        return None;
    }

    let threshold = stop_config
        .map(|s| s.learning_review_threshold)
        .unwrap_or(5);

    // Validate threshold > 0 when enabled
    if threshold == 0 {
        eprintln!(
            "cas: ERROR - learning_review_threshold must be > 0 when learning_review_enabled is true"
        );
        let mut error_context = String::new();
        error_context.push_str("<learning-review-error>\n");
        error_context.push_str("## Learning Review Configuration Error\n\n");
        error_context.push_str("The `learning_review_threshold` is set to 0, which is invalid.\n");
        error_context
            .push_str("Please set `hooks.stop.learning_review_threshold` to a value > 0.\n");
        error_context.push_str("</learning-review-error>\n");
        return Some(error_context);
    }

    // Get unreviewed learnings
    // Fail explicitly if store query fails (no silent skip)
    let unreviewed = match store.list_unreviewed_learnings(LEARNING_REVIEW_LIMIT) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("cas: ERROR - Failed to get unreviewed learnings: {e}");
            // Return error context to block stop and surface the error
            let mut error_context = String::new();
            error_context.push_str("<learning-review-error>\n");
            error_context.push_str("## Learning Review Error\n\n");
            error_context.push_str(&format!(
                "Failed to check for unreviewed learnings: {e}\n\n"
            ));
            error_context.push_str(
                "The `learning_review_enabled` config option is enabled but an error occurred.\n",
            );
            error_context.push_str("Please investigate the store error before stopping.\n");
            error_context.push_str("</learning-review-error>\n");
            return Some(error_context);
        }
    };

    // Check threshold
    if unreviewed.len() < threshold {
        return None;
    }

    // Build learning review context
    let mut context = String::new();
    context.push_str("<learning-review required=\"true\">\n");
    context.push_str(&format!(
        "## Unreviewed Learnings ({} entries)\n\n",
        unreviewed.len()
    ));
    context.push_str("Before closing this session, review these learnings and determine if any should be promoted to rules or skills:\n\n");
    context.push_str("| ID | Created | Content Preview |\n");
    context.push_str("|-----|---------|----------------|\n");

    for entry in unreviewed.iter().take(10) {
        let preview = entry.preview(60);
        let created = entry.created.format("%Y-%m-%d").to_string();
        context.push_str(&format!("| {} | {} | {} |\n", entry.id, created, preview));
    }

    if unreviewed.len() > 10 {
        context.push_str(&format!(
            "\n*...and {} more learnings*\n",
            unreviewed.len() - 10
        ));
    }

    context.push_str("\n**Instructions:**\n");
    context.push_str(
        "1. Use the Task tool to spawn a `learning-reviewer` subagent with this prompt:\n",
    );
    context.push_str("   \"Review the unreviewed learnings in CAS. For each:\n");
    context.push_str("   - If it describes a pattern/convention → create a draft rule\n");
    context.push_str("   - If it describes a workflow/procedure → create a draft skill\n");
    context.push_str("   - If it's project-specific context → leave as learning\n");
    context.push_str("   Mark each learning as reviewed after processing.\"\n");
    context.push_str("2. After the subagent completes, you may stop.\n");
    context.push_str("</learning-review>\n");

    Some(context)
}

/// Build context for rule review when draft rules exceed threshold
///
/// Returns Some(context) if rule review should be triggered, None otherwise.
/// The context instructs the agent to spawn a rule-reviewer subagent.
/// Returns error context if store operations fail (fail explicitly, no silent skip).
pub fn build_rule_review_context(rule_store: &dyn RuleStore, config: &Config) -> Option<String> {
    // Check if rule review is enabled
    let stop_config = config.hooks.as_ref().map(|h| &h.stop);

    let enabled = stop_config.map(|s| s.rule_review_enabled).unwrap_or(false);

    if !enabled {
        return None;
    }

    let threshold = stop_config.map(|s| s.rule_review_threshold).unwrap_or(5);

    // Validate threshold > 0 when enabled
    if threshold == 0 {
        eprintln!(
            "cas: ERROR - rule_review_threshold must be > 0 when rule_review_enabled is true"
        );
        let mut error_context = String::new();
        error_context.push_str("<rule-review-error>\n");
        error_context.push_str("## Rule Review Configuration Error\n\n");
        error_context.push_str("The `rule_review_threshold` is set to 0, which is invalid.\n");
        error_context.push_str("Please set `hooks.stop.rule_review_threshold` to a value > 0.\n");
        error_context.push_str("</rule-review-error>\n");
        return Some(error_context);
    }

    // Get draft rules
    // Fail explicitly if store query fails (no silent skip)
    let all_rules = match rule_store.list() {
        Ok(rules) => rules,
        Err(e) => {
            eprintln!("cas: ERROR - Failed to get rules: {e}");
            // Return error context to block stop and surface the error
            let mut error_context = String::new();
            error_context.push_str("<rule-review-error>\n");
            error_context.push_str("## Rule Review Error\n\n");
            error_context.push_str(&format!("Failed to check for draft rules: {e}\n\n"));
            error_context.push_str(
                "The `rule_review_enabled` config option is enabled but an error occurred.\n",
            );
            error_context.push_str("Please investigate the store error before stopping.\n");
            error_context.push_str("</rule-review-error>\n");
            return Some(error_context);
        }
    };

    let draft_rules: Vec<_> = all_rules
        .iter()
        .filter(|r| r.status == RuleStatus::Draft)
        .collect();

    if draft_rules.len() < threshold {
        return None;
    }

    // Build rule review context
    let mut context = String::new();
    context.push_str("<rule-review required=\"true\">\n");
    context.push_str(&format!(
        "## Draft Rules Pending Review ({} rules)\n\n",
        draft_rules.len()
    ));
    context.push_str(
        "Review these draft rules and determine which should be promoted, merged, or archived:\n\n",
    );
    context.push_str("| ID | Content Preview | Helpful |\n");
    context.push_str("|----|-----------------|----------|\n");

    for rule in draft_rules.iter().take(10) {
        let preview: String = rule.content.chars().take(50).collect();
        let preview = preview.replace('\n', " ");
        context.push_str(&format!(
            "| {} | {}... | {} |\n",
            rule.id, preview, rule.helpful_count
        ));
    }

    if draft_rules.len() > 10 {
        context.push_str(&format!(
            "\n*...and {} more draft rules*\n",
            draft_rules.len() - 10
        ));
    }

    context.push_str("\n**Instructions:**\n");
    context
        .push_str("1. Use the Task tool to spawn a `rule-reviewer` subagent with this prompt:\n");
    context.push_str("   \"Review the draft rules in CAS. For each:\n");
    context.push_str("   - If clear and validated → promote to proven\n");
    context.push_str("   - If similar to another → merge them\n");
    context.push_str("   - If vague or outdated → archive it\"\n");
    context.push_str("2. After the subagent completes, you may stop.\n");
    context.push_str("</rule-review>\n");

    Some(context)
}

/// Build context for duplicate detection when entries exceed threshold
///
/// Returns Some(context) if duplicate detection should be triggered, None otherwise.
/// The context instructs the agent to spawn a duplicate-detector subagent.
/// Returns error context if store operations fail (fail explicitly, no silent skip).
pub fn build_duplicate_detection_context(store: &dyn Store, config: &Config) -> Option<String> {
    // Check if duplicate detection is enabled
    let stop_config = config.hooks.as_ref().map(|h| &h.stop);

    let enabled = stop_config
        .map(|s| s.duplicate_detection_enabled)
        .unwrap_or(false);

    if !enabled {
        return None;
    }

    let threshold = stop_config
        .map(|s| s.duplicate_detection_threshold)
        .unwrap_or(20);

    // Validate threshold > 0 when enabled
    if threshold == 0 {
        eprintln!(
            "cas: ERROR - duplicate_detection_threshold must be > 0 when duplicate_detection_enabled is true"
        );
        let mut error_context = String::new();
        error_context.push_str("<duplicate-detection-error>\n");
        error_context.push_str("## Duplicate Detection Configuration Error\n\n");
        error_context
            .push_str("The `duplicate_detection_threshold` is set to 0, which is invalid.\n");
        error_context
            .push_str("Please set `hooks.stop.duplicate_detection_threshold` to a value > 0.\n");
        error_context.push_str("</duplicate-detection-error>\n");
        return Some(error_context);
    }

    // Count recent entries
    // Fail explicitly if store query fails (no silent skip)
    let recent = match store.list() {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("cas: ERROR - Failed to get entries: {e}");
            // Return error context to block stop and surface the error
            let mut error_context = String::new();
            error_context.push_str("<duplicate-detection-error>\n");
            error_context.push_str("## Duplicate Detection Error\n\n");
            error_context.push_str(&format!("Failed to check for entries: {e}\n\n"));
            error_context.push_str(
                "The `duplicate_detection_enabled` config option is enabled but an error occurred.\n",
            );
            error_context.push_str("Please investigate the store error before stopping.\n");
            error_context.push_str("</duplicate-detection-error>\n");
            return Some(error_context);
        }
    };

    if recent.len() < threshold {
        return None;
    }

    // Build duplicate detection context
    let mut context = String::new();
    context.push_str("<duplicate-detection required=\"true\">\n");
    context.push_str(&format!(
        "## Memory Cleanup Recommended ({} entries)\n\n",
        recent.len()
    ));
    context.push_str("Check for duplicate or near-duplicate entries that can be consolidated:\n\n");
    context.push_str("| ID | Type | Content Preview |\n");
    context.push_str("|----|------|----------------|\n");

    for entry in recent.iter().take(15) {
        let preview = entry.preview(40);
        let entry_type = format!("{:?}", entry.entry_type);
        context.push_str(&format!(
            "| {} | {} | {} |\n",
            entry.id, entry_type, preview
        ));
    }

    context.push_str("\n**Instructions:**\n");
    context.push_str(
        "1. Use the Task tool to spawn a `duplicate-detector` subagent with this prompt:\n",
    );
    context.push_str("   \"Scan CAS memories for duplicates. For each duplicate pair:\n");
    context.push_str("   - Merge content into the more complete entry\n");
    context.push_str("   - Archive the redundant entry\n");
    context.push_str("   - Report statistics on space saved\"\n");
    context.push_str("2. After the subagent completes, you may stop.\n");
    context.push_str("</duplicate-detection>\n");

    Some(context)
}

/// Build context for session summary generation
///
/// Returns Some(context) if generate_summary is enabled and no summary exists yet.
/// The context instructs the agent to spawn a session-summarizer subagent.
/// Returns an error context if store operations fail (fail explicitly, no silent skip).
pub fn build_session_summary_context(
    store: &dyn Store,
    config: &Config,
    session_id: &str,
) -> Option<String> {
    // Check if summary generation is enabled via hooks.stop.generate_summary
    let stop_config = config.hooks.as_ref().map(|h| &h.stop);

    let enabled = stop_config.map(|s| s.generate_summary).unwrap_or(false);

    if !enabled {
        return None;
    }

    // Check if a summary already exists for this session
    // Fail explicitly if store query fails (no silent skip)
    let session_entries = match store.list_by_session(session_id) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("cas: ERROR - Failed to check for existing summaries: {e}");
            // Return error context to block stop and surface the error
            let mut error_context = String::new();
            error_context.push_str("<session-summary-error>\n");
            error_context.push_str("## Session Summary Error\n\n");
            error_context.push_str(&format!(
                "Failed to check for existing session summaries: {e}\n\n"
            ));
            error_context.push_str(
                "The `generate_summary` config option is enabled but an error occurred.\n",
            );
            error_context.push_str("Please investigate the store error before stopping.\n");
            error_context.push_str("</session-summary-error>\n");
            return Some(error_context);
        }
    };

    let has_summary = session_entries.iter().any(|e| {
        e.tags
            .iter()
            .any(|t| t == "session-summary" || t == "summary")
    });

    if has_summary {
        // Summary already exists, don't re-trigger
        return None;
    }

    // Build session summary context
    let mut context = String::new();
    context.push_str("<session-summary required=\"true\">\n");
    context.push_str("## Session Summary Required\n\n");
    context.push_str(
        "Before ending this session, generate a summary of the work done, decisions made, and learnings captured.\n\n",
    );
    context.push_str("**Instructions:**\n");
    context.push_str(
        "1. Use the Task tool to spawn a `session-summarizer` subagent with this prompt:\n",
    );
    context.push_str("   \"Generate a session summary. Steps:\n");
    context.push_str("   - Get session context using mcp__cas__search action=context\n");
    context.push_str("   - List tasks worked on using mcp__cas__task action=mine\n");
    context.push_str("   - Get recent memories using mcp__cas__memory action=recent limit=20\n");
    context.push_str("   - Create a structured summary covering: completed work, decisions, files changed, learnings, blockers, next steps\n");
    context.push_str("   - Store the summary using mcp__cas__memory action=remember with tags session,summary\"\n");
    context.push_str("2. After the subagent completes, you may stop.\n");
    context.push_str("</session-summary>\n");

    Some(context)
}

/// Handle Stop hook - generate session summary when agent finishes
///
/// This is triggered when the agent explicitly finishes its task,
/// making it ideal for generating a comprehensive session summary.
///
/// Implements "compaction" pattern from context engineering:
/// - Summarizes session activities
/// - Preserves architectural decisions
/// - Captures unresolved issues for future sessions
mod stop_flow;
mod synthesis;

pub use stop_flow::handle_stop;
pub use synthesis::synthesize_buffered_observations;
