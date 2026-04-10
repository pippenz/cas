/**
 * Entry point for a per-user daemon (cas-bridge@<username>).
 *
 * Runs as the target Linux user. Listens on a Unix socket for messages
 * forwarded by the router, injects them into CAS via cas serve HTTP API.
 */

import { WebClient } from "@slack/web-api";
import {
  loadDaemonEnv,
  injectMessage,
  startSocketServer,
  type DaemonConfig,
  type MessageHandler,
} from "./daemon.js";
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

  console.log(`Per-user daemon for ${username}`);
  console.log(`  Socket: ${config.socket_path}`);
  console.log(`  CAS serve: ${config.cas_serve_url}`);

  const handleMessage: MessageHandler = async (msg: DaemonMessage) => {
    console.log(
      `[${msg.project}] Message from ${msg.slack_user}: ${msg.text.slice(0, 80)}...`,
    );

    const result = await injectMessage(config, msg);

    if (result.ok) {
      // Confirm injection to the Slack thread
      await slack.chat.postMessage({
        channel: msg.channel,
        thread_ts: msg.thread_ts,
        text: `Message sent to CAS session \`${result.session}\` (message #${result.message_id})`,
      });
    } else {
      // Report error back to Slack thread
      await slack.chat.postMessage({
        channel: msg.channel,
        thread_ts: msg.thread_ts,
        text: `Failed to send to CAS: ${result.error}`,
      });
    }
  };

  const server = startSocketServer(config.socket_path, handleMessage);

  // Graceful shutdown
  const shutdown = () => {
    console.log("Shutting down...");
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
