use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use serde_json::Value;

fn read_jsonl(path: &std::path::Path) -> Vec<Value> {
    let data = std::fs::read_to_string(path).expect("jsonl should be written");
    data.lines()
        .map(|line| serde_json::from_str(line).expect("line should be valid json"))
        .collect()
}

#[allow(deprecated)]
fn cas_cmd() -> Command {
    Command::cargo_bin("cas").unwrap()
}

fn write_jsonl(path: &std::path::Path, values: &[Value]) {
    let body = values
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("{body}\n")).unwrap();
}

fn write_adapter_artifacts(root: &std::path::Path) {
    let claude = root.join("claude");
    let codex = root.join("codex");
    let grok = root.join("grok");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::create_dir_all(&grok).unwrap();

    std::fs::write(
        claude.join("inbox.json"),
        serde_json::to_string_pretty(&serde_json::json!([
            {
                "from": "supervisor",
                "text": "probe-message-id=claude-1 please reply",
                "summary": "probe claude-1",
                "timestamp": "2026-07-21T17:00:01.000Z",
                "color": "green",
                "read": false
            }
        ]))
        .unwrap(),
    )
    .unwrap();
    write_jsonl(
        &claude.join("transcript.jsonl"),
        &[
            serde_json::json!({"timestamp":"2026-07-21T17:00:01.200Z","type":"assistant","message":"ack probe-message-id=claude-1"}),
        ],
    );
    write_jsonl(
        &codex.join("rollout.jsonl"),
        &[
            serde_json::json!({"timestamp":"2026-07-21T17:00:00.000Z","type":"session_meta","payload":{"cwd":"/tmp/probe"}}),
            serde_json::json!({"timestamp":"2026-07-21T17:00:02.000Z","type":"event_msg","payload":{"type":"user_message","message":"probe-message-id=codex-1"}}),
            serde_json::json!({"timestamp":"2026-07-21T17:00:02.100Z","type":"event_msg","payload":{"type":"turn_started"}}),
            serde_json::json!({"timestamp":"2026-07-21T17:00:02.200Z","type":"response_item","payload":{"type":"message","content":"ack probe-message-id=codex-1"}}),
        ],
    );
    write_jsonl(
        &grok.join("updates.jsonl"),
        &[
            serde_json::json!({"timestamp":"2026-07-21T17:00:02.000Z","type":"user_message","text":"probe-message-id=grok-1"}),
            serde_json::json!({"timestamp":"2026-07-21T17:00:02.100Z","type":"turn_started"}),
            serde_json::json!({"timestamp":"2026-07-21T17:00:02.200Z","type":"assistant_message","text":"ack probe-message-id=grok-1"}),
        ],
    );
    write_jsonl(
        &grok.join("events.jsonl"),
        &[
            serde_json::json!({"ts":"2026-07-21T17:00:08.000Z","type":"turn_ended","outcome":"completed"}),
        ],
    );
}

#[test]
fn probe_comm_cli_writes_jsonl_with_fake_adapter() {
    let temp = tempfile::tempdir().unwrap();
    let jsonl = temp.path().join("probe.jsonl");

    cas_cmd()
        .args(["factory", "probe-comm", "--jsonl"])
        .arg(&jsonl)
        .assert()
        .success();

    let lines = read_jsonl(&jsonl);
    assert_eq!(lines.len(), 7);
    assert_eq!(lines[0]["scenario"], "startup");
    assert_eq!(lines[1]["scenario"], "serial_10");
    assert_eq!(lines[1]["message_ids"].as_array().unwrap().len(), 10);
}

#[test]
fn probe_comm_cli_returns_nonzero_for_injected_failure() {
    let temp = tempfile::tempdir().unwrap();
    let jsonl = temp.path().join("probe.jsonl");

    cas_cmd()
        .args([
            "factory",
            "probe-comm",
            "--jsonl",
            jsonl.to_str().unwrap(),
            "--inject-transport-failure",
            "urgent:urgent-0",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("urgent").and(predicates::str::contains("delivered")));

    let lines = read_jsonl(&jsonl);
    let urgent = lines
        .iter()
        .find(|line| line["scenario"] == "urgent")
        .expect("urgent scenario should be written");
    assert_eq!(urgent["failed_stage"], "delivered");
}

#[test]
fn probe_comm_cli_all_adapters_writes_recorded_fixture_report() {
    let temp = tempfile::tempdir().unwrap();
    let jsonl = temp.path().join("probe.jsonl");
    let artifacts = temp.path().join("artifacts");
    write_adapter_artifacts(&artifacts);

    cas_cmd()
        .args([
            "factory",
            "probe-comm",
            "--jsonl",
            jsonl.to_str().unwrap(),
            "--adapter",
            "all",
            "--artifact-root",
            artifacts.to_str().unwrap(),
        ])
        .assert()
        .success();

    let lines = read_jsonl(&jsonl);
    let scenarios: Vec<_> = lines
        .iter()
        .map(|line| line["scenario"].as_str().unwrap())
        .collect();
    assert!(
        scenarios.contains(&"routing_matrix_evidence"),
        "report must compose routing matrix evidence from cas-4484: {scenarios:?}"
    );
    assert!(
        scenarios.contains(&"merge_reclose_lifecycle_evidence"),
        "report must compose merge/re-close lifecycle evidence from cas-126b/cas-062d: {scenarios:?}"
    );
    assert!(
        lines
            .iter()
            .filter(|line| line["scenario"].as_str().unwrap().ends_with("_adapter"))
            .all(|line| line["passed"] == true)
    );
    assert!(
        lines
            .iter()
            .filter(|line| line["scenario"].as_str().unwrap().ends_with("_adapter"))
            .all(|line| line["stages"][0]["reaction_status"] == "OBSERVED")
    );
    assert!(
        lines
            .iter()
            .filter(|line| line["scenario"].as_str().unwrap().ends_with("_adapter"))
            .all(|line| line["stages"][0]["enqueued_at_ms"].is_null()
                && line["stages"][0]["selected_at_ms"].is_null()),
        "recorded adapters must not fabricate enqueue/select timestamps: {lines:?}"
    );
    let routing = lines
        .iter()
        .find(|line| line["scenario"] == "routing_matrix_evidence")
        .unwrap();
    assert_eq!(
        routing["stages"].as_array().unwrap().len(),
        19,
        "18 bidirectional harness pairings plus one live-observation disclosure"
    );
    assert!(
        routing["stages"].as_array().unwrap().iter().any(|stage| {
            stage["message_id"] == "live-disposable-model-observation"
                && stage["stage_statuses"][0]["status"] == "BLOCKED"
                && stage["reaction_status"] == "BLOCKED"
        }),
        "unsupported live observation must be BLOCKED/UNKNOWN, never pass-like: {routing}"
    );
}

#[test]
fn probe_comm_cli_recorded_adapter_applies_slo_thresholds() {
    let temp = tempfile::tempdir().unwrap();
    let jsonl = temp.path().join("probe.jsonl");
    let artifacts = temp.path().join("artifacts");
    write_adapter_artifacts(&artifacts);

    cas_cmd()
        .args([
            "factory",
            "probe-comm",
            "--jsonl",
            jsonl.to_str().unwrap(),
            "--adapter",
            "codex",
            "--artifact-root",
            artifacts.to_str().unwrap(),
            "--delivery-slo-ms",
            "1",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("reaction_slo"));

    let lines = read_jsonl(&jsonl);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["scenario"], "codex_adapter");
    assert_eq!(lines[0]["passed"], false);
    assert_eq!(lines[0]["failed_stage"], "reaction_slo");
    assert_eq!(lines[0]["stages"][0]["terminal"], "reaction_slo_failed");
}

#[test]
fn probe_comm_cli_malformed_recorded_artifact_emits_stage_status_jsonl() {
    let temp = tempfile::tempdir().unwrap();
    let jsonl = temp.path().join("probe.jsonl");
    let artifacts = temp.path().join("artifacts");
    let codex = artifacts.join("codex");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::write(codex.join("rollout.jsonl"), "{not-json}\n").unwrap();

    cas_cmd()
        .args([
            "factory",
            "probe-comm",
            "--jsonl",
            jsonl.to_str().unwrap(),
            "--adapter",
            "codex",
            "--artifact-root",
            artifacts.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("artifact_parse_failed"));

    let lines = read_jsonl(&jsonl);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["scenario"], "codex_adapter");
    assert_eq!(lines[0]["passed"], false);
    assert_eq!(lines[0]["failed_stage"], "artifact_parse_failed");
    assert_eq!(lines[0]["stages"][0]["terminal"], "artifact_parse_failed");
    assert_eq!(lines[0]["stages"][0]["delivered_status"], "FAILED");
    assert!(
        lines[0]["stages"][0]["stage_statuses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stage| stage["stage"] == "artifact" && stage["status"] == "FAILED")
    );
}

#[test]
fn probe_comm_cli_missing_recorded_artifact_emits_stage_status_jsonl() {
    let temp = tempfile::tempdir().unwrap();
    let jsonl = temp.path().join("probe.jsonl");
    let artifacts = temp.path().join("artifacts");
    std::fs::create_dir_all(&artifacts).unwrap();

    cas_cmd()
        .args([
            "factory",
            "probe-comm",
            "--jsonl",
            jsonl.to_str().unwrap(),
            "--adapter",
            "codex",
            "--artifact-root",
            artifacts.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("artifact_missing"));

    let lines = read_jsonl(&jsonl);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["scenario"], "codex_adapter");
    assert_eq!(lines[0]["failed_stage"], "artifact_missing");
    assert_eq!(lines[0]["stages"][0]["terminal"], "artifact_missing");
    assert_eq!(lines[0]["stages"][0]["delivered_status"], "FAILED");
}

#[test]
fn probe_comm_cli_unmatched_recorded_artifact_emits_unknown_stage_status_jsonl() {
    let temp = tempfile::tempdir().unwrap();
    let jsonl = temp.path().join("probe.jsonl");
    let artifacts = temp.path().join("artifacts");
    let codex = artifacts.join("codex");
    std::fs::create_dir_all(&codex).unwrap();
    write_jsonl(
        &codex.join("rollout.jsonl"),
        &[serde_json::json!({
            "timestamp":"2026-07-21T17:00:02.000Z",
            "type":"event_msg",
            "payload":{"type":"user_message","message":"probe-message-id=other"}
        })],
    );

    cas_cmd()
        .args([
            "factory",
            "probe-comm",
            "--jsonl",
            jsonl.to_str().unwrap(),
            "--adapter",
            "codex",
            "--artifact-root",
            artifacts.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("correlation_unknown"));

    let lines = read_jsonl(&jsonl);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["scenario"], "codex_adapter");
    assert_eq!(lines[0]["failed_stage"], "correlation_unknown");
    assert_eq!(lines[0]["stages"][0]["terminal"], "correlation_unknown");
    assert_eq!(lines[0]["stages"][0]["delivered_status"], "UNKNOWN");
    assert!(
        lines[0]["stages"][0]["stage_statuses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stage| stage["stage"] == "artifact" && stage["status"] == "UNKNOWN")
    );
}
