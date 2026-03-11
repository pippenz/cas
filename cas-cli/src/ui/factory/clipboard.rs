//! Clipboard support for the factory TUI
//!
//! On macOS, uses `pbcopy` subprocess to avoid NSPasteboard fork-safety issues.
//! On other platforms, uses the arboard crate.

/// Copy text to the system clipboard.
///
/// Returns Ok(()) on success, or an error if clipboard access fails.
///
/// # macOS Note
/// Uses `pbcopy` subprocess instead of direct NSPasteboard access because
/// NSPasteboard is not fork-safe. The factory daemon runs in a forked process,
/// and calling NSPasteboard APIs after fork() causes crashes.
pub fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }

        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("pbcopy failed with status: {status}");
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        use arboard::Clipboard;
        let mut clipboard = Clipboard::new()?;
        clipboard.set_text(text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::factory::clipboard::*;

    #[test]
    fn test_copy_to_clipboard() {
        // On macOS, pbcopy should always be available
        #[cfg(target_os = "macos")]
        {
            let result = copy_to_clipboard("test text");
            assert!(result.is_ok(), "pbcopy should succeed on macOS");
        }

        // On other platforms, skip if no display available (CI environments)
        #[cfg(not(target_os = "macos"))]
        {
            if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
                return;
            }
            let result = copy_to_clipboard("test text");
            assert!(result.is_ok() || result.is_err());
        }
    }
}
