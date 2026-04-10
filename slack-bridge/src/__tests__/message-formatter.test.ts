import { describe, it, expect } from "vitest";
import {
  formatActivityEvent,
  formatInboxMessage,
  formatError,
  formatInjectionConfirm,
} from "../message-formatter.js";

describe("formatActivityEvent", () => {
  it("formats supervisor_injected event", () => {
    const result = formatActivityEvent({
      id: 100,
      event_type: "supervisor_injected",
      entity_type: "agent",
      entity_id: "noble-owl-1",
      summary: "Injected queued prompt 920 to noble-owl-1 (ok)",
      metadata: { prompt_id: 920, status: "ok" },
      created_at: "2026-04-10T16:00:00Z",
    });
    expect(result).toBeNull(); // injection confirmations are noise, skip them
  });

  it("formats task_completed event as readable text", () => {
    const result = formatActivityEvent({
      id: 200,
      event_type: "task_completed",
      entity_type: "task",
      entity_id: "cas-1234",
      summary: "Task cas-1234 completed: Build hello world file",
      metadata: null,
      created_at: "2026-04-10T16:05:00Z",
    });
    expect(result).not.toBeNull();
    expect(result!.text).toContain("cas-1234");
    expect(result!.text).toContain("completed");
  });

  it("formats generic event using summary field", () => {
    const result = formatActivityEvent({
      id: 300,
      event_type: "tool_use",
      entity_type: "agent",
      entity_id: "worker-1",
      summary: "worker-1 edited src/main.rs",
      metadata: null,
      created_at: "2026-04-10T16:10:00Z",
    });
    expect(result).not.toBeNull();
    expect(result!.text).toContain("worker-1 edited src/main.rs");
  });

  it("strips ANSI codes from summaries", () => {
    const result = formatActivityEvent({
      id: 400,
      event_type: "tool_use",
      entity_type: "agent",
      entity_id: "worker-1",
      summary: "\u001b[32mworker-1\u001b[0m edited file",
      metadata: null,
      created_at: "2026-04-10T16:15:00Z",
    });
    expect(result).not.toBeNull();
    expect(result!.text).not.toContain("\u001b");
    expect(result!.text).toContain("worker-1");
  });
});

describe("formatInboxMessage", () => {
  it("formats supervisor response as a quote block", () => {
    const result = formatInboxMessage({
      id: 50,
      from: "noble-owl-1",
      to: "slack-1234.5678",
      payload: "I've created the hello world file at src/hello.rs",
      created_at: "2026-04-10T16:20:00Z",
    });
    expect(result.text).toContain("hello world file");
    expect(result.blocks).toBeDefined();
    expect(result.blocks!.length).toBeGreaterThan(0);
  });
});

describe("formatError", () => {
  it("formats error with code and message", () => {
    const result = formatError("session_not_found", "Session 'foo' not found");
    expect(result.text).toContain("session_not_found");
    expect(result.text).toContain("Session 'foo' not found");
  });
});

describe("formatInjectionConfirm", () => {
  it("formats injection confirmation", () => {
    const result = formatInjectionConfirm("my-session", 42);
    expect(result.text).toContain("my-session");
    expect(result.text).toContain("42");
  });
});
