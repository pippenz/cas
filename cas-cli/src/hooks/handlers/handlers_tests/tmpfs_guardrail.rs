use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::hooks::handlers::*;
use crate::hooks::types::HookInput;

#[derive(Default)]
struct FakeUsageReader {
    usage_by_path: HashMap<PathBuf, u64>,
}

impl UsageReader for FakeUsageReader {
    fn usage(&self, path: &Path) -> Option<MountUsage> {
        self.usage_by_path
            .get(path)
            .copied()
            .map(|used_bytes| MountUsage { used_bytes })
    }
}

fn input_for(tool: &str, tool_input: serde_json::Value) -> HookInput {
    HookInput {
        session_id: "tmpfs-session".to_string(),
        cwd: "/mem/project".to_string(),
        hook_event_name: "PostToolUse".to_string(),
        tool_name: Some(tool.to_string()),
        tool_input: Some(tool_input),
        ..HookInput::default()
    }
}

fn mounts() -> Vec<MountInfo> {
    vec![
        MountInfo {
            mount_point: PathBuf::from("/"),
            fs_type: "ext4".to_string(),
        },
        MountInfo {
            mount_point: PathBuf::from("/mem"),
            fs_type: "tmpfs".to_string(),
        },
        MountInfo {
            mount_point: PathBuf::from("/durable"),
            fs_type: "xfs".to_string(),
        },
    ]
}

#[test]
fn tmpfs_write_crossing_threshold_warns() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = input_for(
        "Write",
        serde_json::json!({"file_path": "/mem/out.bin", "content": "hello world"}),
    );
    let warning = maybe_tmpfs_guardrail_warning_with(
        tmp.path(),
        &input,
        &mounts(),
        &FakeUsageReader::default(),
        10,
        None,
    )
    .expect("tmpfs write over threshold should warn");

    assert!(warning.contains("WARNING"));
    assert!(warning.contains("/mem"));
    assert!(warning.contains("11 bytes"));
    assert!(warning.contains("10 bytes"));
    assert!(warning.contains("warning only"));
}

#[test]
fn non_tmpfs_write_is_ignored() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = input_for(
        "Write",
        serde_json::json!({"file_path": "/durable/out.bin", "content": "hello world"}),
    );

    assert!(
        maybe_tmpfs_guardrail_warning_with(
            tmp.path(),
            &input,
            &mounts(),
            &FakeUsageReader::default(),
            10,
            None
        )
        .is_none()
    );
}

#[test]
fn cumulative_tmpfs_writes_respect_threshold() {
    let tmp = tempfile::TempDir::new().unwrap();
    let first = input_for(
        "Write",
        serde_json::json!({"file_path": "/mem/a.bin", "content": "12345"}),
    );
    let second = input_for(
        "Write",
        serde_json::json!({"file_path": "/mem/b.bin", "content": "67890"}),
    );

    assert!(
        maybe_tmpfs_guardrail_warning_with(
            tmp.path(),
            &first,
            &mounts(),
            &FakeUsageReader::default(),
            10,
            None
        )
        .is_none()
    );
    assert!(
        maybe_tmpfs_guardrail_warning_with(
            tmp.path(),
            &second,
            &mounts(),
            &FakeUsageReader::default(),
            10,
            None
        )
        .is_some()
    );
}

#[test]
fn warning_includes_configured_staging_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = input_for(
        "Write",
        serde_json::json!({"file_path": "/mem/out.bin", "content": "hello world"}),
    );
    let warning = maybe_tmpfs_guardrail_warning_with(
        tmp.path(),
        &input,
        &mounts(),
        &FakeUsageReader::default(),
        10,
        Some("/durable/cas-staging"),
    )
    .expect("tmpfs write over threshold should warn");

    assert!(warning.contains("/durable/cas-staging"));
    assert!(warning.contains("approved staging location"));
}

#[test]
fn warns_once_per_threshold_crossing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = input_for(
        "Write",
        serde_json::json!({"file_path": "/mem/out.bin", "content": "hello world"}),
    );

    assert!(
        maybe_tmpfs_guardrail_warning_with(
            tmp.path(),
            &input,
            &mounts(),
            &FakeUsageReader::default(),
            10,
            None
        )
        .is_some()
    );
    assert!(
        maybe_tmpfs_guardrail_warning_with(
            tmp.path(),
            &input,
            &mounts(),
            &FakeUsageReader::default(),
            15,
            None
        )
        .is_none(),
        "same threshold band should not spam repeated warnings"
    );
    assert!(
        maybe_tmpfs_guardrail_warning_with(
            tmp.path(),
            &input,
            &mounts(),
            &FakeUsageReader::default(),
            10,
            None
        )
        .is_some(),
        "next threshold crossing should warn again"
    );
}

#[test]
fn bash_usage_growth_warns_after_baseline() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = input_for("Bash", serde_json::json!({"command": "make artifact"}));
    let mut usage = FakeUsageReader::default();
    usage.usage_by_path.insert(PathBuf::from("/mem"), 100);

    assert!(
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &input, &mounts(), &usage, 10, None)
            .is_none(),
        "first Bash sample establishes the session baseline"
    );

    usage.usage_by_path.insert(PathBuf::from("/mem"), 111);
    let warning =
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &input, &mounts(), &usage, 10, None)
            .expect("usage growth past threshold should warn");
    assert!(warning.contains("11 bytes"));
}

#[test]
fn parse_mounts_decodes_proc_mount_escapes() {
    let parsed = parse_mounts("tmpfs /tmp/a\\040b tmpfs rw,nosuid 0 0\n/dev/sda1 / ext4 rw 0 0\n");
    assert_eq!(
        parsed[0],
        MountInfo {
            mount_point: PathBuf::from("/tmp/a b"),
            fs_type: "tmpfs".to_string(),
        }
    );
    assert_eq!(
        find_mount_for_path(Path::new("/tmp/a b/file"), &parsed)
            .unwrap()
            .mount_point,
        PathBuf::from("/tmp/a b")
    );
}
