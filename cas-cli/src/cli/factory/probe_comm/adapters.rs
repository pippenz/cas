#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_jsonl(path: &std::path::Path, values: &[serde_json::Value]) {
        let body = values
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, format!("{body}\n")).unwrap();
    }

    #[test]
    fn claude_fixture_extracts_inbox_delivery_and_transcript_reaction() {
        let temp = tempfile::tempdir().unwrap();
        let inbox = temp.path().join("worker-a.json");
        let transcript = temp.path().join("claude-session.jsonl");
        std::fs::write(
            &inbox,
            serde_json::to_string_pretty(&json!([
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
            &transcript,
            &[
                json!({"timestamp":"2026-07-21T17:00:04.000Z","type":"assistant","message":"ack probe-message-id=claude-1"}),
            ],
        );

        let got = ClaudeAdapter::extract_fixture(&ClaudeFixture {
            inbox_path: inbox,
            transcript_path: Some(transcript),
            message_id: "claude-1".to_string(),
            target: "worker-a".to_string(),
        })
        .expect("claude fixture should parse");

        assert_eq!(got.message_id, "claude-1");
        assert_eq!(got.target, "worker-a");
        assert_eq!(got.delivered_at_ms, Some(1_000));
        assert_eq!(got.wake_at_ms, Some(1_000));
        assert_eq!(got.first_reaction_at_ms, Some(4_000));
        assert_eq!(got.reaction_status.as_deref(), Some("OBSERVED"));
        assert_eq!(got.terminal, "delivered");
    }

    #[test]
    fn codex_fixture_correlates_rollout_turn_and_reaction() {
        let temp = tempfile::tempdir().unwrap();
        let rollout = temp.path().join("rollout.jsonl");
        write_jsonl(
            &rollout,
            &[
                json!({"timestamp":"2026-07-21T17:00:00.000Z","type":"session_meta","payload":{"cwd":"/tmp/probe"}}),
                json!({"timestamp":"2026-07-21T17:00:02.000Z","type":"event_msg","payload":{"type":"user_message","message":"probe-message-id=codex-1"}}),
                json!({"timestamp":"2026-07-21T17:00:03.000Z","type":"event_msg","payload":{"type":"turn_started"}}),
                json!({"timestamp":"2026-07-21T17:00:06.000Z","type":"response_item","payload":{"type":"message","content":"ack probe-message-id=codex-1"}}),
            ],
        );

        let got = CodexAdapter::extract_fixture(&CodexFixture {
            rollout_path: rollout,
            message_id: "codex-1".to_string(),
            target: "worker-a".to_string(),
        })
        .expect("codex fixture should parse");

        assert_eq!(got.delivered_at_ms, Some(2_000));
        assert_eq!(got.wake_at_ms, Some(3_000));
        assert_eq!(got.first_reaction_at_ms, Some(6_000));
        assert_eq!(got.reaction_status.as_deref(), Some("OBSERVED"));
    }

    #[test]
    fn grok_fixture_uses_updates_and_events_without_inventing_reaction() {
        let temp = tempfile::tempdir().unwrap();
        let updates = temp.path().join("updates.jsonl");
        let events = temp.path().join("events.jsonl");
        write_jsonl(
            &updates,
            &[
                json!({"timestamp":"2026-07-21T17:00:02.000Z","type":"user_message","text":"probe-message-id=grok-1"}),
                json!({"timestamp":"2026-07-21T17:00:03.000Z","type":"turn_started"}),
            ],
        );
        write_jsonl(
            &events,
            &[json!({"ts":"2026-07-21T17:00:08.000Z","type":"turn_ended","outcome":"completed"})],
        );

        let got = GrokAdapter::extract_fixture(&GrokFixture {
            updates_path: updates,
            events_path: Some(events),
            message_id: "grok-1".to_string(),
            target: "worker-a".to_string(),
        })
        .expect("grok fixture should parse");

        assert_eq!(got.delivered_at_ms, Some(2_000));
        assert_eq!(got.wake_at_ms, Some(3_000));
        assert_eq!(got.first_reaction_at_ms, None);
        assert_eq!(got.reaction_status.as_deref(), Some("UNKNOWN"));
        assert_eq!(got.terminal, "delivered");
    }
}
