/**
 * Entry point for the router service (cas-bridge-router).
 *
 * Runs as the unprivileged `cas-bridge` system user.
 * Connects to Slack via Socket Mode, routes messages to per-user daemons.
 */

import { App } from "@slack/bolt";
import { loadConfig } from "./config.js";
import { registerMessageHandler } from "./router.js";

async function main(): Promise<void> {
  // Require Slack tokens
  const botToken = process.env.SLACK_BOT_TOKEN;
  const appToken = process.env.SLACK_APP_TOKEN;
  if (!botToken || !appToken) {
    console.error(
      "Missing required env vars: SLACK_BOT_TOKEN, SLACK_APP_TOKEN",
    );
    process.exit(1);
  }

  const config = loadConfig();
  console.log(
    `Loaded config: ${Object.keys(config.channels).length} channels, ${Object.keys(config.users).length} users`,
  );

  const app = new App({
    token: botToken,
    socketMode: true,
    appToken,
  });

  registerMessageHandler(app, config);

  await app.start();
  console.log("CAS Slack bridge router is running (Socket Mode)");

  // Graceful shutdown
  const shutdown = async () => {
    console.log("Shutting down...");
    await app.stop();
    process.exit(0);
  };
  process.on("SIGTERM", shutdown);
  process.on("SIGINT", shutdown);
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
