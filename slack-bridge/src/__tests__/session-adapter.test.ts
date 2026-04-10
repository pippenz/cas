import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  parseSseChunk,
  SseEventBuffer,
  buildAuthHeaders,
} from "../session-adapter.js";

describe("parseSseChunk", () => {
  it("parses a complete SSE event", () => {
    const raw = "event: activity\ndata: {\"schema_version\":1}\n\n";
    const events = parseSseChunk(raw);
    expect(events).toHaveLength(1);
    expect(events[0].event).toBe("activity");
    expect(events[0].data).toBe('{"schema_version":1}');
  });

  it("parses multi-line data fields", () => {
    const raw = "event: status\ndata: line1\ndata: line2\n\n";
    const events = parseSseChunk(raw);
    expect(events).toHaveLength(1);
    expect(events[0].data).toBe("line1\nline2");
  });

  it("ignores SSE comments", () => {
    const raw = ": heartbeat\n\n";
    const events = parseSseChunk(raw);
    expect(events).toHaveLength(0);
  });

  it("ignores the initial connected comment", () => {
    const raw = ": connected\n\nevent: activity\ndata: {}\n\n";
    const events = parseSseChunk(raw);
    expect(events).toHaveLength(1);
    expect(events[0].event).toBe("activity");
  });

  it("parses multiple events in one chunk", () => {
    const raw =
      "event: activity\ndata: {\"a\":1}\n\nevent: status\ndata: {\"b\":2}\n\n";
    const events = parseSseChunk(raw);
    expect(events).toHaveLength(2);
    expect(events[0].event).toBe("activity");
    expect(events[1].event).toBe("status");
  });

  it("handles empty data field", () => {
    const raw = "event: ping\ndata:\n\n";
    const events = parseSseChunk(raw);
    expect(events).toHaveLength(1);
    expect(events[0].data).toBe("");
  });
});

describe("SseEventBuffer", () => {
  it("buffers partial chunks and emits on complete event", () => {
    const buffer = new SseEventBuffer();
    // First chunk: incomplete event
    const events1 = buffer.push("event: activity\n");
    expect(events1).toHaveLength(0);
    // Second chunk: completes the event
    const events2 = buffer.push("data: {}\n\n");
    expect(events2).toHaveLength(1);
    expect(events2[0].event).toBe("activity");
  });

  it("handles multiple events split across chunks", () => {
    const buffer = new SseEventBuffer();
    const e1 = buffer.push("event: a\ndata: 1\n\nevent: b\n");
    expect(e1).toHaveLength(1);
    const e2 = buffer.push("data: 2\n\n");
    expect(e2).toHaveLength(1);
    expect(e2[0].event).toBe("b");
  });
});

describe("buildAuthHeaders", () => {
  it("includes bearer token when provided", () => {
    const headers = buildAuthHeaders("my-token");
    expect(headers["Authorization"]).toBe("Bearer my-token");
  });

  it("omits auth header when token is empty", () => {
    const headers = buildAuthHeaders("");
    expect(headers["Authorization"]).toBeUndefined();
  });
});
