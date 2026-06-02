mod agent_worktree_block;
mod ask_user_question_remind;
mod basic;
mod factory_auto_approve;
mod message_display;
mod permission_request_factory;
mod preferences_context;
mod reviews;
mod ripple_path_scope;
mod send_message_autoroute;
mod supervisor_reminder;

/// Process-wide mutex for tests that mutate `CAS_AGENT_ROLE` (or any other
/// env var read by the PreToolUse / PermissionRequest handlers).
///
/// All submodules that call `std::env::set_var("CAS_AGENT_ROLE", …)` must
/// hold this guard for the duration of the test.  Using per-module mutexes
/// silently fails: they don't coordinate with each other, so two tests in
/// different modules can race on the same env var.
///
/// Usage in a submodule: `let _g = super::env_lock();`
pub(super) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}
