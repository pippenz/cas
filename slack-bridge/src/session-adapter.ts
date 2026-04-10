/**
 * Session adapter: SSE subscription and message injection for cas serve.
 *
 * Manages the SSE connection to a CAS factory session, parses events,
 * and provides helpers for message injection. Reconnects on connection drop.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface SseEvent {
  event: string;
  data: string;
}

export interface SessionSubscription {
  /** Abort the SSE connection */
  abort: () => void;
}

export type SseEventHandler = (event: SseEvent) => void;

// ---------------------------------------------------------------------------
// SSE parsing
// ---------------------------------------------------------------------------

/**
 * Parse raw SSE text into discrete events.
 * Handles: `event:`, `data:`, and comment lines (`: ...`).
 * Events are separated by blank lines (`\n\n`).
 */
export function parseSseChunk(raw: string): SseEvent[] {
  const events: SseEvent[] = [];
  // Split on double newline (event boundary)
  const blocks = raw.split("\n\n");

  for (const block of blocks) {
    const trimmed = block.trim();
    if (!trimmed) continue;

    let eventName = "";
    const dataLines: string[] = [];
    let isComment = true;

    for (const line of trimmed.split("\n")) {
      if (line.startsWith(": ") || line === ":") {
        // SSE comment — skip
        continue;
      }
      isComment = false;
      if (line.startsWith("event: ") || line.startsWith("event:")) {
        eventName = line.slice(line.indexOf(":") + 1).trim();
      } else if (line.startsWith("data: ") || line.startsWith("data:")) {
        dataLines.push(line.slice(line.indexOf(":") + 1).trimStart());
      }
    }

    if (!isComment && eventName) {
      events.push({
        event: eventName,
        data: dataLines.join("\n"),
      });
    }
  }

  return events;
}

/**
 * Buffer for handling SSE data that arrives in arbitrary chunks.
 * Accumulates text and emits complete events.
 */
export class SseEventBuffer {
  private buffer = "";

  /** Push a chunk of data and return any complete events. */
  push(chunk: string): SseEvent[] {
    this.buffer += chunk;
    const events: SseEvent[] = [];

    // Look for complete events (terminated by \n\n)
    while (true) {
      const idx = this.buffer.indexOf("\n\n");
      if (idx === -1) break;

      const block = this.buffer.slice(0, idx);
      this.buffer = this.buffer.slice(idx + 2);

      const parsed = parseSseChunk(block + "\n\n");
      events.push(...parsed);
    }

    return events;
  }

  /** Reset the buffer. */
  clear(): void {
    this.buffer = "";
  }
}

// ---------------------------------------------------------------------------
// Auth headers
// ---------------------------------------------------------------------------

/** Build HTTP headers for cas serve requests. */
export function buildAuthHeaders(
  token: string,
): Record<string, string> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }
  return headers;
}

// ---------------------------------------------------------------------------
// SSE subscription
// ---------------------------------------------------------------------------

/**
 * Subscribe to a CAS session's SSE event stream.
 *
 * Opens a long-lived connection to `GET /v1/sessions/{name}/events`.
 * Calls `onEvent` for each parsed SSE event.
 * Automatically reconnects on connection drop with exponential backoff.
 *
 * Returns a handle to abort the subscription.
 */
export function subscribeToSession(
  casServeUrl: string,
  sessionName: string,
  token: string,
  sinceId: number,
  onEvent: SseEventHandler,
  onError?: (err: Error) => void,
): SessionSubscription {
  const controller = new AbortController();
  let lastId = sinceId;
  let reconnectDelay = 1000;
  const maxReconnectDelay = 30_000;

  async function connect(): Promise<void> {
    if (controller.signal.aborted) return;

    const url = new URL(
      `/v1/sessions/${sessionName}/events`,
      casServeUrl,
    );
    url.searchParams.set("since_id", String(lastId));
    url.searchParams.set("poll_ms", "500");
    url.searchParams.set("heartbeat_ms", "15000");
    url.searchParams.set("activity_limit", "20");
    url.searchParams.set("inbox_limit", "10");
    url.searchParams.set("include_status", "false");
    url.searchParams.set("inbox_id", "owner");

    const headers: Record<string, string> = {};
    if (token) {
      headers["Authorization"] = `Bearer ${token}`;
    }

    try {
      const res = await fetch(url.toString(), {
        headers,
        signal: controller.signal,
      });

      if (!res.ok || !res.body) {
        throw new Error(`SSE connect failed: ${res.status}`);
      }

      // Reset backoff on successful connection
      reconnectDelay = 1000;

      const buffer = new SseEventBuffer();
      const reader = res.body.getReader();
      const decoder = new TextDecoder();

      while (true) {
        const { done, value } = await reader.read();
        if (done || controller.signal.aborted) break;

        const chunk = decoder.decode(value, { stream: true });
        const events = buffer.push(chunk);

        for (const ev of events) {
          // Track latest activity ID for reconnect
          if (ev.event === "activity") {
            try {
              const data = JSON.parse(ev.data);
              if (data.latest_id && data.latest_id > lastId) {
                lastId = data.latest_id;
              }
            } catch {
              // ignore parse errors in ID tracking
            }
          }
          onEvent(ev);
        }
      }
    } catch (err) {
      if (controller.signal.aborted) return;

      const error = err instanceof Error ? err : new Error(String(err));
      onError?.(error);

      // Reconnect with exponential backoff
      console.error(
        `SSE connection lost: ${error.message}. Reconnecting in ${reconnectDelay}ms...`,
      );
      await new Promise((r) => setTimeout(r, reconnectDelay));
      reconnectDelay = Math.min(reconnectDelay * 2, maxReconnectDelay);
      connect();
    }
  }

  // Start the connection
  connect();

  return {
    abort: () => controller.abort(),
  };
}
