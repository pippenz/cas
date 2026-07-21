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
