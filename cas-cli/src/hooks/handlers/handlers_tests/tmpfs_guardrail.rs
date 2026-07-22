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
            mount_point: PathBuf::from("/dev/shm"),
            fs_type: "tmpfs".to_string(),
        },
        MountInfo {
            mount_point: PathBuf::from("/durable"),
            fs_type: "xfs".to_string(),
        },
    ]
}

#[test]
fn edit_new_string_crossing_threshold_warns() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = input_for(
        "Edit",
        serde_json::json!({
            "file_path": "/mem/out.bin",
            "old_string": "",
            "new_string": "hello world"
        }),
    );
    let warning = maybe_tmpfs_guardrail_warning_with(
        tmp.path(),
        &input,
        &mounts(),
        &FakeUsageReader::default(),
        10,
        None,
    )
    .expect("tmpfs edit over threshold should warn");

    assert!(warning.contains("11 bytes"));
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
    usage.usage_by_path.insert(PathBuf::from("/mem"), 5);

    assert!(
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &input, &mounts(), &usage, 10, None)
            .is_none(),
        "first Bash sample establishes the session baseline"
    );

    usage.usage_by_path.insert(PathBuf::from("/mem"), 16);
    let warning =
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &input, &mounts(), &usage, 10, None)
            .expect("usage growth past threshold should warn");
    assert!(warning.contains("11 bytes"));
}

#[test]
fn bash_first_sample_over_threshold_warns_immediately() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = input_for("Bash", serde_json::json!({"command": "make artifact"}));
    let mut usage = FakeUsageReader::default();
    usage.usage_by_path.insert(PathBuf::from("/mem"), 17);

    let warning =
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &input, &mounts(), &usage, 10, None)
            .expect("single-shot tmpfs fill should warn on first Bash sample");
    assert!(warning.contains("17 bytes"));
}

#[test]
fn write_and_bash_accounting_do_not_clobber_each_other() {
    let tmp = tempfile::TempDir::new().unwrap();
    let write_16 = input_for(
        "Write",
        serde_json::json!({"file_path": "/mem/a.bin", "content": "1234567890123456"}),
    );
    let write_4 = input_for(
        "Write",
        serde_json::json!({"file_path": "/mem/b.bin", "content": "1234"}),
    );
    let bash = input_for("Bash", serde_json::json!({"command": "make artifact"}));
    let mut usage = FakeUsageReader::default();

    assert!(
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &write_16, &mounts(), &usage, 10, None)
            .is_some()
    );

    usage.usage_by_path.insert(PathBuf::from("/mem"), 1);
    assert!(
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &bash, &mounts(), &usage, 10, None)
            .is_none()
    );

    usage.usage_by_path.insert(PathBuf::from("/mem"), 5);
    assert!(
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &bash, &mounts(), &usage, 10, None)
            .is_none()
    );

    let warning =
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &write_4, &mounts(), &usage, 10, None)
            .expect("written bytes should still reach the second threshold");
    assert!(warning.contains("20 bytes"));
}

#[test]
fn write_warning_does_not_suppress_bash_usage_warning() {
    let tmp = tempfile::TempDir::new().unwrap();
    let write = input_for(
        "Write",
        serde_json::json!({"file_path": "/mem/a.bin", "content": "123456789012"}),
    );
    let bash = input_for("Bash", serde_json::json!({"command": "make artifact"}));
    let mut usage = FakeUsageReader::default();

    assert!(
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &write, &mounts(), &usage, 10, None)
            .is_some()
    );

    usage.usage_by_path.insert(PathBuf::from("/mem"), 12);
    let warning =
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &bash, &mounts(), &usage, 10, None)
            .expect("usage-growth threshold should warn even after a write warning");
    assert!(warning.contains("12 bytes"));
}

#[test]
fn bash_samples_all_tmpfs_mounts_not_just_cwd_and_tmp() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = input_for("Bash", serde_json::json!({"command": "make artifact"}));
    let mut usage = FakeUsageReader::default();
    usage.usage_by_path.insert(PathBuf::from("/mem"), 0);
    usage.usage_by_path.insert(PathBuf::from("/dev/shm"), 12);

    let warning =
        maybe_tmpfs_guardrail_warning_with(tmp.path(), &input, &mounts(), &usage, 10, None)
            .expect("/dev/shm fill should be sampled and warned");
    assert!(warning.contains("/dev/shm"));
    assert!(warning.contains("12 bytes"));
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
