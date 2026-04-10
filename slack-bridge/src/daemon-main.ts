/**
 * Entry point for a per-user daemon (cas-bridge@<username>).
 *
 * Runs as the target Linux user. Listens on a Unix socket for messages
 * forwarded by the router, injects them into CAS via cas serve HTTP API.
 *
 * Session lifecycle: lazy startup, idle timeout, shutdown/reset commands.
 */

import { WebClient } from "@slack/web-api";
import {
  loadDaemonEnv,
  injectMessage,
  startSocketServer,
  type DaemonConfig,
  type MessageHandler,
} from "./daemon.js";
import { SessionManager, type SessionEntry } from "./session-manager.js";
import { detectCommand, executeCommand } from "./commands.js";
import type { DaemonMessage } from "./router.js";

async function main(): Promise<void> {
  // Load env from ~/.config/cas/env
  const env = loadDaemonEnv();

  const username = process.env.CAS_BRIDGE_USER ?? process.env.USER ?? "unknown";
  const socketPath =
    process.env.CAS_BRIDGE_SOCKET ??
    `/run/cas-bridge/${username}.sock`;

  const config: DaemonConfig = {
    socket_path: socketPath,
    username,
    cas_serve_url:
      env.CAS_SERVE_URL ?? process.env.CAS_SERVE_URL ?? "http://127.0.0.1:18999",
    cas_serve_token: env.CAS_SERVE_TOKEN ?? process.env.CAS_SERVE_TOKEN ?? "",
    slack_bot_token:
      env.SLACK_BOT_TOKEN ?? process.env.SLACK_BOT_TOKEN ?? "",
  };

  if (!config.slack_bot_token) {
    console.error("Missing SLACK_BOT_TOKEN in env");
    process.exit(1);
  }

  const slack = new WebClient(config.slack_bot_token);

  // Parse idle timeout from env (minutes, default 30)
  const idleTimeoutMin = parseInt(
    env.CAS_IDLE_TIMEOUT_MIN ?? process.env.CAS_IDLE_TIMEOUT_MIN ?? "30",
    10,
  );

  const sessionManager = new SessionManager(config, {
    idle_timeout_ms: idleTimeoutMin * 60 * 1000,
  });

  console.log(`Per-user daemon for ${username}`);
  console.log(`  Socket: ${config.socket_path}`);
  console.log(`  CAS serve: ${config.cas_serve_url}`);
  console.log(`  Idle timeout: ${idleTimeoutMin}m`);

  // Startup cleanup: adopt orphaned sessions
  const cleanup = await sessionManager.cleanupOrphans();
  if (cleanup.adopted.length > 0) {
    console.log(`Adopted ${cleanup.adopted.length} orphaned session(s): ${cleanup.adopted.join(", ")}`);
  }
  if (cleanup.killed.length > 0) {
    console.log(`Killed ${cleanup.killed.length} orphaned session(s): ${cleanup.killed.join(", ")}`);
  }

  // Start idle timeout checker
  sessionManager.startIdleChecker(async (entry: SessionEntry, warning: boolean) => {
    // We don't have a channel/thread reference here — log only.
    // A future iteration could store the last Slack channel+thread per session
    // and post warnings there.
    if (warning) {
      console.log(`Session ${entry.name} idle — will shut down in 5m`);
    } else {
      console.log(`Session ${entry.name} timed out — killed`);
    }
  });

  const handleMessage: MessageHandler = async (msg: DaemonMessage) => {
    console.log(
      `[${msg.project}] Message from ${msg.slack_user}: ${msg.text.slice(0, 80)}...`,
    );

    // Check for lifecycle commands first
    const command = detectCommand(msg.text);
    if (command) {
      const result = await executeCommand(command, sessionManager, msg.project_dir);
      if (result.handled) {
        await slack.chat.postMessage({
          channel: msg.channel,
          thread_ts: msg.thread_ts,
          text: result.reply,
        });
        return;
      }
    }

    // Ensure a session exists (lazy startup)
    const session = await sessionManager.ensureSession(msg.project_dir);
    if (!session) {
      await slack.chat.postMessage({
        channel: msg.channel,
        thread_ts: msg.thread_ts,
        text: "Failed to start CAS session. Check server logs.",
      });
      return;
    }

    if (session.started) {
      await slack.chat.postMessage({
        channel: msg.channel,
        thread_ts: msg.thread_ts,
        text: `Starting CAS factory in ${msg.project}... ready. Session: \`${session.name}\``,
      });
    }

    // Touch the session to reset idle timer
    sessionManager.touch(msg.project_dir);

    // Inject the message
    const result = await injectMessage(config, msg);

    if (result.ok) {
      await slack.chat.postMessage({
        channel: msg.channel,
        thread_ts: msg.thread_ts,
        text: `Message sent to CAS session \`${result.session}\` (message #${result.message_id})`,
      });
    } else {
      await slack.chat.postMessage({
        channel: msg.channel,
        thread_ts: msg.thread_ts,
        text: `Failed to send to CAS: ${result.error}`,
      });
    }
  };

  const server = startSocketServer(config.socket_path, handleMessage);

  // Graceful shutdown
  const shutdown = async () => {
    console.log("Shutting down...");
    sessionManager.stopIdleChecker();
    server.close();
    process.exit(0);
  };
  process.on("SIGTERM", shutdown);
  process.on("SIGINT", shutdown);
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
