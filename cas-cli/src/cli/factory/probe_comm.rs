#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn read_jsonl(path: &std::path::Path) -> Vec<Value> {
        let data = std::fs::read_to_string(path).expect("jsonl should be written");
        data.lines()
            .map(|line| serde_json::from_str(line).expect("line should be valid json"))
            .collect()
    }

    fn base_config(jsonl: std::path::PathBuf, cas_root: std::path::PathBuf) -> ProbeCommConfig {
        ProbeCommConfig {
            jsonl,
            cas_root: Some(cas_root),
            allow_active_cas_root: true,
            adapter: Some(ProbeAdapterKind::Fake),
            thresholds: ProbeThresholds::epic_defaults(),
            failure: None,
        }
    }

    #[test]
    fn fake_adapter_happy_path_writes_all_stage_evidence_jsonl() {
        let temp = tempfile::tempdir().unwrap();
        let jsonl = temp.path().join("probe.jsonl");

        let report = run_probe_comm(base_config(jsonl.clone(), temp.path().join(".cas")))
            .expect("happy path should pass");

        assert!(report.passed, "report should pass: {report:?}");
        assert_eq!(report.scenarios.len(), 7);
        assert!(report.scenarios.iter().all(|scenario| scenario.passed));

        let lines = read_jsonl(&jsonl);
        assert_eq!(lines.len(), 7);

        let names: Vec<_> = lines
            .iter()
            .map(|line| line["scenario"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "startup",
                "serial_10",
                "fifo_burst",
                "urgent",
                "duplicate_replay",
                "malformed_target",
                "lifecycle",
            ]
        );

        let serial = lines
            .iter()
            .find(|line| line["scenario"] == "serial_10")
            .expect("serial scenario should be present");
        assert_eq!(serial["message_ids"].as_array().unwrap().len(), 10);
        for stage in serial["stages"].as_array().unwrap() {
            assert!(stage["enqueued_at_ms"].as_u64().is_some());
            assert!(stage["selected_at_ms"].as_u64().is_some());
            assert!(stage["delivered_at_ms"].as_u64().is_some());
        }
    }

    #[test]
    fn fifo_and_duplicate_replay_assertions_are_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        let jsonl = temp.path().join("probe.jsonl");

        let report = run_probe_comm(base_config(jsonl.clone(), temp.path().join(".cas")))
            .expect("happy path should pass");
        let fifo = report
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "fifo_burst")
            .unwrap();
        assert_eq!(fifo.message_ids, vec!["fifo-0", "fifo-1", "fifo-2", "fifo-3", "fifo-4"]);

        let replay = report
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "duplicate_replay")
            .unwrap();
        assert_eq!(replay.message_ids, vec!["replay-original"]);

        let lines = read_jsonl(&jsonl);
        let replay_json = lines
            .iter()
            .find(|line| line["scenario"] == "duplicate_replay")
            .unwrap();
        assert_eq!(replay_json["duplicates_suppressed"], 1);
    }

    #[test]
    fn active_parent_cas_root_is_rejected_without_override() {
        let temp = tempfile::tempdir().unwrap();
        let jsonl = temp.path().join("probe.jsonl");
        let parent_root = temp.path().join(".cas");
        std::fs::create_dir_all(&parent_root).unwrap();

        let mut config = base_config(jsonl, parent_root.clone());
        config.allow_active_cas_root = false;

        let err = run_probe_comm_with_parent(config, Some(&parent_root))
            .expect_err("active parent root should be rejected");
        assert!(err.to_string().contains("active parent CAS root"));
    }

    #[test]
    fn missing_adapter_and_malformed_output_path_fail_clearly() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = base_config(temp.path().join("probe.jsonl"), temp.path().join(".cas"));
        config.adapter = None;
        let err = run_probe_comm(config).expect_err("missing adapter should fail");
        assert!(err.to_string().contains("probe adapter is required"));

        let dir_output = temp.path().join("jsonl-dir");
        std::fs::create_dir_all(&dir_output).unwrap();
        let config = base_config(dir_output, temp.path().join(".cas"));
        let err = run_probe_comm(config).expect_err("directory output path should fail");
        assert!(err.to_string().contains("jsonl output path"));
    }

    #[test]
    fn injected_transport_and_slo_failures_return_failed_stage() {
        let temp = tempfile::tempdir().unwrap();

        let mut transport = base_config(temp.path().join("transport.jsonl"), temp.path().join(".cas"));
        transport.failure = Some(ProbeFailure::Transport {
            scenario: "urgent".to_string(),
            message_id: "urgent-0".to_string(),
        });
        let err = run_probe_comm(transport).expect_err("transport failure should fail run");
        assert!(err.to_string().contains("urgent"));
        assert!(err.to_string().contains("delivered"));

        let mut slo = base_config(temp.path().join("slo.jsonl"), temp.path().join(".cas"));
        slo.thresholds.delivery_ms = 1;
        slo.failure = Some(ProbeFailure::Slo {
            scenario: "serial_10".to_string(),
            message_id: "serial-0".to_string(),
            delivered_after_ms: 50,
        });
        let err = run_probe_comm(slo).expect_err("SLO failure should fail run");
        assert!(err.to_string().contains("serial_10"));
        assert!(err.to_string().contains("delivery_slo"));
    }
}
