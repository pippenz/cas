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
import { spawn as spawnChild } from "node:child_process";
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
 * Execute a Claude Code command directly in the project directory.
 *
 * Spawns `claude -p "message" --dangerously-skip-permissions` as a child
 * process and captures its output. This bypasses the factory/PTY pipeline
 * which has timing issues with Claude Code's TUI initialization.
 */
/** Track which Slack threads already have a Claude session started. */
const threadSessions = new Set<string>();

/**
 * Convert a Slack thread_ts to a deterministic UUID-like session ID.
 * Claude Code requires a valid UUID format for --session-id.
 */
function threadTsToSessionId(threadTs: string): string {
  // Pad/hash the thread_ts into a stable 32-hex-char string
  const hex = Buffer.from(threadTs.padEnd(32, "0")).toString("hex").slice(0, 32);
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20, 32)}`;
}

export async function injectMessage(
  config: DaemonConfig,
  msg: DaemonMessage,
): Promise<{ ok: boolean; session?: string; message_id?: number; response?: string; error?: string }> {
  return new Promise((resolve) => {
    const sessionId = threadTsToSessionId(msg.thread_ts);
    const isResume = threadSessions.has(msg.thread_ts);

    const args = [
      "--dangerously-skip-permissions",
      "-p", msg.text,
      "--effort", "high",
      "--max-turns", "20",
    ];

    if (isResume) {
      // Resume existing session — adds new message to the conversation
      args.push("--resume", sessionId);
    } else {
      // New session — set session ID so we can resume later
      args.push("--session-id", sessionId);
    }

    console.log(`Spawning claude [${isResume ? "resume" : "new"}] in ${msg.project_dir}: ${msg.text.slice(0, 60)}`);

    const child = spawnChild("claude", args, {
      cwd: msg.project_dir,
      env: { ...process.env, HOME: process.env.HOME },
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 300_000, // 5 minute timeout
    });

    // Track this thread for future resume
    threadSessions.add(msg.thread_ts);

    let stdout = "";
    let stderr = "";

    child.stdout?.on("data", (chunk: Buffer) => {
      stdout += chunk.toString();
    });

    child.stderr?.on("data", (chunk: Buffer) => {
      stderr += chunk.toString();
    });

    child.on("close", (code) => {
      if (code === 0 && stdout.trim()) {
        resolve({
          ok: true,
          session: "direct",
          message_id: Date.now(),
          response: stdout.trim(),
        });
      } else {
        resolve({
          ok: false,
          error: `claude exited ${code}: ${stderr.slice(0, 200) || stdout.slice(0, 200) || "no output"}`,
        });
      }
    });

    child.on("error", (err) => {
      resolve({
        ok: false,
        error: `spawn failed: ${err.message}`,
      });
    });
  });
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
