//! SSH-based remote attach to factories on other machines.
//!
//! Resolves device name → SSH host via cloud API, builds an SSH command,
//! and replaces the current process with `ssh -t <host> -- cas attach <factory-id>`.

use std::io;

use anyhow::{Result, bail};

use crate::cloud::CloudConfig;
use crate::ui::components::{Formatter, Renderable, StatusLine};
use crate::ui::theme::ActiveTheme;

/// Parse a remote target string into (device_name, factory_id).
///
/// Formats:
///   - `device:factory-id` → (Some("device"), "factory-id")
///   - `factory-id`        → (None, "factory-id")
fn parse_remote_target(target: &str) -> (Option<&str>, &str) {
    if let Some(pos) = target.find(':') {
        let device = &target[..pos];
        let factory_id = &target[pos + 1..];
        if device.is_empty() {
            (None, factory_id)
        } else {
            (Some(device), factory_id)
        }
    } else {
        (None, target)
    }
}

/// Look up a device's SSH host from the cloud API.
///
/// Fetches GET /api/devices and finds the device by name or ID,
/// then extracts the ssh_config.host field.
fn resolve_ssh_host(cloud_config: &CloudConfig, device_name: &str) -> Result<String> {
    let url = format!("{}/api/devices", cloud_config.endpoint);
    let token = cloud_config.token.as_deref().unwrap_or("");

    let response = ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => {
                anyhow::anyhow!("Authentication failed. Run `cas login` first.")
            }
            e => anyhow::anyhow!("Failed to fetch devices: {e}"),
        })?;

    let body: serde_json::Value = response.into_json()?;
    let devices = body["devices"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid response from cloud API"))?;

    // Find device by name or ID
    let device = devices
        .iter()
        .find(|d| d["name"].as_str() == Some(device_name) || d["id"].as_str() == Some(device_name))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Device '{device_name}' not found. Run `cas device list` to see registered devices."
            )
        })?;

    // Extract ssh_config.host
    let ssh_host = device["ssh_config"]["host"]
        .as_str()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No SSH config for device '{device_name}'. Set it with: cas device ssh-config {device_name} user@hostname"
            )
        })?;

    Ok(ssh_host.to_string())
}

/// Execute remote attach via SSH.
///
/// 1. Parse target into device name and factory ID
/// 2. Look up device SSH host from cloud API
/// 3. Build SSH command: `ssh -t <host> -- cas attach <factory-id> [--worker <name>]`
/// 4. Replace current process with SSH via exec
pub fn execute_remote_attach(target: &str, worker: Option<&str>) -> Result<()> {
    let (device_name, factory_id) = parse_remote_target(target);

    if factory_id.is_empty() {
        bail!("Factory ID is required. Usage: cas attach --remote [device:]factory-id");
    }

    // Resolve SSH host
    let ssh_host = match device_name {
        Some(name) => {
            let cloud_config = CloudConfig::load().unwrap_or_default();
            if !cloud_config.is_logged_in() {
                bail!("Not logged in. Run `cas login` first.");
            }
            resolve_ssh_host(&cloud_config, name)?
        }
        None => {
            bail!(
                "Device name required for remote attach.\n\
                 Usage: cas attach --remote <device>:<factory-id>\n\n\
                 Run `cas device list` to see registered devices."
            );
        }
    };

    // Build remote cas attach command
    let mut remote_args = vec![factory_id.to_string()];
    if let Some(w) = worker {
        remote_args.push("--worker".to_string());
        remote_args.push(w.to_string());
    }

    let remote_cmd = format!("cas attach {}", remote_args.join(" "));

    {
        let theme = ActiveTheme::default();
        let mut stdout = io::stdout();
        let mut fmt = Formatter::stdout(&mut stdout, theme);
        StatusLine::info(format!("Connecting to {ssh_host} via SSH...")).render(&mut fmt)?;
    }

    // Build SSH args: ssh -t <host> -- cas attach <factory-id> [--worker <name>]
    let ssh_args = vec![
        "ssh".to_string(),
        "-t".to_string(),
        ssh_host,
        "--".to_string(),
        remote_cmd,
    ];

    // Replace current process with SSH
    exec_ssh(&ssh_args)
}

/// Replace the current process with SSH using exec.
///
/// On Unix, uses execvp to replace the process entirely.
/// On non-Unix, spawns SSH as a child process.
fn exec_ssh(args: &[String]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::ffi::CString;

        let c_args: Vec<CString> = args
            .iter()
            .map(|a| CString::new(a.as_bytes()).unwrap())
            .collect();

        // execvp replaces the current process
        nix::unistd::execvp(&c_args[0], &c_args).map_err(|e| {
            anyhow::anyhow!(
                "Failed to exec SSH. Is ssh installed?\n\
                 Error: {e}\n\n\
                 Cloud relay mode (Phase 2b) is not yet available as a fallback."
            )
        })?;

        unreachable!()
    }

    #[cfg(not(unix))]
    {
        let status = std::process::Command::new(&args[0])
            .args(&args[1..])
            .status()
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to launch SSH: {}\n\n\
                     Cloud relay mode (Phase 2b) is not yet available as a fallback.",
                    e
                )
            })?;

        if !status.success() {
            bail!(
                "SSH exited with status: {}\n\n\
                 If the remote machine is unreachable, cloud relay mode (Phase 2b) \
                 will be available in a future update.",
                status
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_remote_target_with_device() {
        let (device, factory_id) = parse_remote_target("desktop:f-abc123");
        assert_eq!(device, Some("desktop"));
        assert_eq!(factory_id, "f-abc123");
    }

    #[test]
    fn test_parse_remote_target_without_device() {
        let (device, factory_id) = parse_remote_target("f-abc123");
        assert_eq!(device, None);
        assert_eq!(factory_id, "f-abc123");
    }

    #[test]
    fn test_parse_remote_target_empty_device() {
        let (device, factory_id) = parse_remote_target(":f-abc123");
        assert_eq!(device, None);
        assert_eq!(factory_id, "f-abc123");
    }

    #[test]
    fn test_parse_remote_target_complex_factory_id() {
        let (device, factory_id) = parse_remote_target("laptop:my-session-name");
        assert_eq!(device, Some("laptop"));
        assert_eq!(factory_id, "my-session-name");
    }
}
