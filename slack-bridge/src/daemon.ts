/**
 * Per-user daemon: Runs as a specific Linux user (e.g., cas-bridge@daniel).
 *
 * Listens on a Unix socket for forwarded Slack messages from the router,
 * injects them into the appropriate CAS factory session via `cas serve` HTTP API,
 * and streams responses back to the originating Slack thread via SSE.
 *
 * Reads CAS credentials from ~/.config/cas/env.
 */

import { createServer, type Server, type Socket } from "node:net";
import { readFileSync, existsSync, mkdirSync, unlinkSync, chmodSync } from "node:fs";
import { resolve } from "node:path";
import type { DaemonMessage } from "./router.js";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface DaemonConfig {
  /** Unix socket path this daemon listens on */
  socket_path: string;
  /** Linux username this daemon runs as */
  username: string;
  /** cas serve base URL (e.g., http://127.0.0.1:18999) */
  cas_serve_url: string;
  /** Bearer token for cas serve auth (empty if --no-auth) */
  cas_serve_token: string;
  /** Slack bot token for posting replies */
  slack_bot_token: string;
}

// ---------------------------------------------------------------------------
// Env loading
// ---------------------------------------------------------------------------

/**
 * Load daemon environment from ~/.config/cas/env.
 * Format: KEY=VALUE lines, # comments, empty lines ignored.
 */
export function loadDaemonEnv(
  envPath?: string,
): Record<string, string> {
  const path =
    envPath ??
    process.env.CAS_BRIDGE_ENV ??
    resolve(process.env.HOME ?? "/tmp", ".config/cas/env");

  const vars: Record<string, string> = {};
  if (!existsSync(path)) {
    console.warn(`Env file not found at ${path}`);
    return vars;
  }

  const lines = readFileSync(path, "utf-8").split("\n");
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eq = trimmed.indexOf("=");
    if (eq < 1) continue;
    const key = trimmed.slice(0, eq).trim();
    const val = trimmed.slice(eq + 1).trim();
    vars[key] = val;
  }
  return vars;
}

// ---------------------------------------------------------------------------
// CAS serve client
// ---------------------------------------------------------------------------

/**
 * Inject a message into a CAS factory session via the cas serve HTTP API.
 *
 * Uses: POST /v1/sessions/<session>/message
 *   { target: "supervisor", message: text, from: "slack-<thread_ts>" }
 *
 * If no session exists for the project, tries to start one via:
 *   POST /v1/factory/start { project_dir, reuse_existing: true }
 */
export async function injectMessage(
  config: DaemonConfig,
  msg: DaemonMessage,
): Promise<{ ok: boolean; session?: string; message_id?: number; error?: string }> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (config.cas_serve_token) {
    headers["Authorization"] = `Bearer ${config.cas_serve_token}`;
  }

  // Step 1: Find or start a session for this project
  const startRes = await fetch(`${config.cas_serve_url}/v1/factory/start`, {
    method: "POST",
    headers,
    body: JSON.stringify({
      project_dir: msg.project_dir,
      reuse_existing: true,
      workers: 0, // supervisor-only for now
    }),
  });

  if (!startRes.ok) {
    const body = await startRes.text();
    return { ok: false, error: `factory/start failed: ${startRes.status} ${body}` };
  }

  const startData = (await startRes.json()) as {
    session: { name: string };
    started: boolean;
    reused_existing: boolean;
  };
  const sessionName = startData.session.name;

  // Step 2: Inject the message
  const msgRes = await fetch(
    `${config.cas_serve_url}/v1/sessions/${sessionName}/message`,
    {
      method: "POST",
      headers,
      body: JSON.stringify({
        target: "supervisor",
        message: msg.text,
        from: `slack-${msg.thread_ts}`,
        no_wrap: false,
        wait_ack: false,
      }),
    },
  );

  if (!msgRes.ok) {
    const body = await msgRes.text();
    return { ok: false, session: sessionName, error: `message inject failed: ${msgRes.status} ${body}` };
  }

  const msgData = (await msgRes.json()) as {
    message_id: number;
    enqueued: boolean;
  };

  return {
    ok: true,
    session: sessionName,
    message_id: msgData.message_id,
  };
}

// ---------------------------------------------------------------------------
// Unix socket server
// ---------------------------------------------------------------------------

export type MessageHandler = (msg: DaemonMessage) => Promise<void>;

/**
 * Start the Unix socket server that receives forwarded messages from the router.
 */
export function startSocketServer(
  socketPath: string,
  onMessage: MessageHandler,
): Server {
  // Clean up stale socket
  if (existsSync(socketPath)) {
    unlinkSync(socketPath);
  }

  // Ensure parent directory exists
  const dir = socketPath.slice(0, socketPath.lastIndexOf("/"));
  if (dir && !existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
  }

  const server = createServer((conn: Socket) => {
    let data = "";

    conn.on("data", (chunk) => {
      data += chunk.toString();
    });

    conn.on("end", () => {
      // Each connection sends one JSON message terminated by newline
      const lines = data.split("\n").filter((l) => l.trim());
      for (const line of lines) {
        try {
          const msg = JSON.parse(line) as DaemonMessage;
          onMessage(msg).catch((err) => {
            console.error(`Handler error: ${err}`);
          });
        } catch (err) {
          console.error(`Invalid JSON from router: ${err}`);
        }
      }
    });

    conn.on("error", (err) => {
      console.error(`Socket connection error: ${err.message}`);
    });
  });

  server.listen(socketPath, () => {
    console.log(`Daemon listening on ${socketPath}`);
  });

  // Make socket world-writable so the unprivileged router can connect
  server.on("listening", () => {
    chmodSync(socketPath, 0o777);
  });

  return server;
}
