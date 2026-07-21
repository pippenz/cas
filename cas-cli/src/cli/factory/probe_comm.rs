use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub(crate) mod adapters;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ProbeAdapterKind {
    Fake,
    Claude,
    Codex,
    Grok,
    All,
}

#[derive(Clone, Debug)]
pub(crate) struct ProbeThresholds {
    pub delivery_ms: u64,
    pub selection_ms: u64,
}

impl ProbeThresholds {
    #[cfg(test)]
    pub(crate) fn epic_defaults() -> Self {
        Self {
            delivery_ms: 500,
            selection_ms: 250,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProbeFailure {
    Transport {
        scenario: String,
        message_id: String,
    },
    Slo {
        scenario: String,
        message_id: String,
        delivered_after_ms: u64,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct ProbeCommConfig {
    pub jsonl: PathBuf,
    pub cas_root: Option<PathBuf>,
    pub allow_active_cas_root: bool,
    pub adapter: Option<ProbeAdapterKind>,
    pub thresholds: ProbeThresholds,
    pub failure: Option<ProbeFailure>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ProbeReport {
    pub passed: bool,
    pub scenarios: Vec<ScenarioReport>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ScenarioReport {
    #[serde(rename = "scenario")]
    pub name: &'static str,
    pub passed: bool,
    pub failed_stage: Option<&'static str>,
    pub message_ids: Vec<String>,
    pub duplicates_suppressed: u32,
    pub malformed_targets_rejected: u32,
    pub stages: Vec<MessageStageEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MessageStageEvidence {
    pub message_id: String,
    pub target: String,
    pub enqueued_at_ms: u64,
    pub selected_at_ms: u64,
    pub delivered_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wake_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_reaction_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_status: Option<String>,
    pub terminal: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct ScenarioDef {
    name: &'static str,
    messages: &'static [&'static str],
    duplicate_of: Option<&'static str>,
    malformed_target: bool,
}

const SCENARIOS: &[ScenarioDef] = &[
    ScenarioDef {
        name: "startup",
        messages: &["startup-0"],
        duplicate_of: None,
        malformed_target: false,
    },
    ScenarioDef {
        name: "serial_10",
        messages: &[
            "serial-0", "serial-1", "serial-2", "serial-3", "serial-4", "serial-5", "serial-6",
            "serial-7", "serial-8", "serial-9",
        ],
        duplicate_of: None,
        malformed_target: false,
    },
    ScenarioDef {
        name: "fifo_burst",
        messages: &["fifo-0", "fifo-1", "fifo-2", "fifo-3", "fifo-4"],
        duplicate_of: None,
        malformed_target: false,
    },
    ScenarioDef {
        name: "urgent",
        messages: &["urgent-0"],
        duplicate_of: None,
        malformed_target: false,
    },
    ScenarioDef {
        name: "duplicate_replay",
        messages: &["replay-original"],
        duplicate_of: Some("replay-original"),
        malformed_target: false,
    },
    ScenarioDef {
        name: "malformed_target",
        messages: &["malformed-0"],
        duplicate_of: None,
        malformed_target: true,
    },
    ScenarioDef {
        name: "lifecycle",
        messages: &["lifecycle-start", "lifecycle-stop"],
        duplicate_of: None,
        malformed_target: false,
    },
];

pub(crate) fn execute_probe_comm(
    jsonl: PathBuf,
    cas_root: Option<PathBuf>,
    parent_cas_root: Option<&Path>,
    allow_active_cas_root: bool,
    adapter: ProbeAdapterKind,
    delivery_slo_ms: u64,
    selection_slo_ms: u64,
    failure: Option<ProbeFailure>,
) -> Result<()> {
    let report = run_probe_comm_with_parent(
        ProbeCommConfig {
            jsonl,
            cas_root,
            allow_active_cas_root,
            adapter: Some(adapter),
            thresholds: ProbeThresholds {
                delivery_ms: delivery_slo_ms,
                selection_ms: selection_slo_ms,
            },
            failure,
        },
        parent_cas_root,
    )?;

    println!(
        "probe-comm passed: {} scenarios",
        report
            .scenarios
            .iter()
            .filter(|scenario| scenario.passed)
            .count()
    );
    Ok(())
}

pub(crate) fn parse_probe_failure(
    transport: Option<&str>,
    slo: Option<&str>,
) -> Result<Option<ProbeFailure>> {
    match (transport, slo) {
        (Some(_), Some(_)) => {
            bail!("use only one probe failure injection flag");
        }
        (Some(spec), None) => {
            let (scenario, message_id) = spec
                .split_once(':')
                .context("--inject-transport-failure must be SCENARIO:MESSAGE_ID")?;
            if scenario.is_empty() || message_id.is_empty() {
                bail!("--inject-transport-failure must be SCENARIO:MESSAGE_ID");
            }
            Ok(Some(ProbeFailure::Transport {
                scenario: scenario.to_string(),
                message_id: message_id.to_string(),
            }))
        }
        (None, Some(spec)) => {
            let mut parts = spec.split(':');
            let scenario = parts.next().unwrap_or_default();
            let message_id = parts.next().unwrap_or_default();
            let delivered_after_ms = parts.next().unwrap_or_default();
            if scenario.is_empty()
                || message_id.is_empty()
                || delivered_after_ms.is_empty()
                || parts.next().is_some()
            {
                bail!("--inject-slo-failure must be SCENARIO:MESSAGE_ID:MS");
            }
            Ok(Some(ProbeFailure::Slo {
                scenario: scenario.to_string(),
                message_id: message_id.to_string(),
                delivered_after_ms: delivered_after_ms
                    .parse()
                    .context("--inject-slo-failure MS must be an integer")?,
            }))
        }
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
pub(crate) fn run_probe_comm(config: ProbeCommConfig) -> Result<ProbeReport> {
    run_probe_comm_with_parent(config, None)
}

pub(crate) fn run_probe_comm_with_parent(
    config: ProbeCommConfig,
    parent_cas_root: Option<&Path>,
) -> Result<ProbeReport> {
    let ProbeCommConfig {
        jsonl,
        cas_root,
        allow_active_cas_root,
        adapter,
        thresholds,
        failure,
    } = config;

    let adapter = adapter.context("probe adapter is required")?;
    match adapter {
        ProbeAdapterKind::Fake => {}
    }

    if jsonl.is_dir() {
        bail!("jsonl output path is a directory: {}", jsonl.display());
    }
    if let Some(parent) = jsonl.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create jsonl output directory {}",
                parent.display()
            )
        })?;
    }

    let run_root = match cas_root {
        Some(path) => path,
        None => std::env::temp_dir()
            .join(format!("cas-probe-comm-{}", uuid::Uuid::new_v4()))
            .join(".cas"),
    };
    guard_active_parent_root(&run_root, parent_cas_root, allow_active_cas_root)?;
    fs::create_dir_all(&run_root)
        .with_context(|| format!("failed to create isolated CAS root {}", run_root.display()))?;

    let mut writer = BufWriter::new(
        File::create(&jsonl)
            .with_context(|| format!("failed to create jsonl output path {}", jsonl.display()))?,
    );

    let mut clock = FakeClock::default();
    let mut scenarios = Vec::with_capacity(SCENARIOS.len());
    for scenario in SCENARIOS {
        let report = run_scenario(*scenario, &thresholds, failure.as_ref(), &mut clock);
        serde_json::to_writer(&mut writer, &report)
            .context("failed to serialize probe scenario evidence")?;
        writer
            .write_all(b"\n")
            .context("failed to write probe scenario evidence")?;
        scenarios.push(report);
    }
    writer
        .flush()
        .context("failed to flush probe scenario evidence")?;

    let report = ProbeReport {
        passed: scenarios.iter().all(|scenario| scenario.passed),
        scenarios,
    };

    if let Some(failed) = report.scenarios.iter().find(|scenario| !scenario.passed) {
        bail!(
            "probe-comm scenario '{}' failed at stage '{}'",
            failed.name,
            failed.failed_stage.unwrap_or("unknown")
        );
    }

    Ok(report)
}

fn guard_active_parent_root(
    run_root: &Path,
    parent_cas_root: Option<&Path>,
    allow_active_cas_root: bool,
) -> Result<()> {
    if allow_active_cas_root {
        return Ok(());
    }

    let Some(parent) = parent_cas_root else {
        return Ok(());
    };

    if equivalent_paths(run_root, parent) {
        bail!(
            "refusing to run probe-comm in the active parent CAS root {}; pass the explicit safe override only for disposable roots",
            run_root.display()
        );
    }

    Ok(())
}

fn equivalent_paths(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn run_scenario(
    scenario: ScenarioDef,
    thresholds: &ProbeThresholds,
    failure: Option<&ProbeFailure>,
    clock: &mut FakeClock,
) -> ScenarioReport {
    let mut stages = Vec::with_capacity(scenario.messages.len());
    let mut failed_stage = None;

    for message_id in scenario.messages {
        let evidence = run_message(scenario, message_id, thresholds, failure, clock);
        if evidence.terminal == "transport_failed" {
            failed_stage = Some("delivered");
        } else if evidence.terminal == "delivery_slo_failed" {
            failed_stage = Some("delivery_slo");
        } else if evidence
            .selected_at_ms
            .saturating_sub(evidence.enqueued_at_ms)
            > thresholds.selection_ms
        {
            failed_stage = Some("selected");
        }
        stages.push(evidence);
    }

    let duplicates_suppressed = u32::from(scenario.duplicate_of.is_some());
    let malformed_targets_rejected = u32::from(scenario.malformed_target);
    let message_ids = stages
        .iter()
        .filter(|stage| stage.terminal == "delivered")
        .map(|stage| stage.message_id.clone())
        .collect();

    ScenarioReport {
        name: scenario.name,
        passed: failed_stage.is_none(),
        failed_stage,
        message_ids,
        duplicates_suppressed,
        malformed_targets_rejected,
        stages,
    }
}

fn run_message(
    scenario: ScenarioDef,
    message_id: &str,
    thresholds: &ProbeThresholds,
    failure: Option<&ProbeFailure>,
    clock: &mut FakeClock,
) -> MessageStageEvidence {
    let enqueued_at_ms = clock.tick(1);
    let selected_at_ms = clock.tick(0);
    let target = if scenario.malformed_target {
        "missing-worker"
    } else {
        "worker-a"
    };

    if scenario.malformed_target {
        return MessageStageEvidence {
            message_id: message_id.to_string(),
            target,
            enqueued_at_ms,
            selected_at_ms,
            delivered_at_ms: None,
            wake_at_ms: None,
            first_reaction_at_ms: None,
            reaction_status: Some("UNKNOWN".to_string()),
            terminal: "target_rejected",
        };
    }

    if matches!(
        failure,
        Some(ProbeFailure::Transport { scenario: s, message_id: id })
            if s == scenario.name && id == message_id
    ) {
        return MessageStageEvidence {
            message_id: message_id.to_string(),
            target,
            enqueued_at_ms,
            selected_at_ms,
            delivered_at_ms: None,
            wake_at_ms: None,
            first_reaction_at_ms: None,
            reaction_status: Some("UNKNOWN".to_string()),
            terminal: "transport_failed",
        };
    }

    let delivery_delta = match failure {
        Some(ProbeFailure::Slo {
            scenario: s,
            message_id: id,
            delivered_after_ms,
        }) if s == scenario.name && id == message_id => *delivered_after_ms,
        _ => thresholds.delivery_ms.min(4),
    };
    let delivered_at_ms = clock.advance_to(enqueued_at_ms + delivery_delta);
    let terminal = if delivered_at_ms.saturating_sub(enqueued_at_ms) > thresholds.delivery_ms {
        "delivery_slo_failed"
    } else {
        "delivered"
    };

    MessageStageEvidence {
        message_id: message_id.to_string(),
        target,
        enqueued_at_ms,
        selected_at_ms,
        delivered_at_ms: Some(delivered_at_ms),
        wake_at_ms: None,
        first_reaction_at_ms: None,
        reaction_status: None,
        terminal,
    }
}

#[derive(Default)]
struct FakeClock {
    now_ms: u64,
}

impl FakeClock {
    fn tick(&mut self, delta_ms: u64) -> u64 {
        self.now_ms += delta_ms;
        self.now_ms
    }

    fn advance_to(&mut self, timestamp_ms: u64) -> u64 {
        self.now_ms = self.now_ms.max(timestamp_ms);
        self.now_ms
    }
}

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
        assert_eq!(
            fifo.message_ids,
            vec!["fifo-0", "fifo-1", "fifo-2", "fifo-3", "fifo-4"]
        );

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

        let mut transport = base_config(
            temp.path().join("transport.jsonl"),
            temp.path().join(".cas"),
        );
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
