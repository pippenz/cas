/**
 * Entry point for a per-user daemon (cas-bridge@<username>).
 *
 * Runs as the target Linux user. Listens on a Unix socket for messages
 * forwarded by the router, injects them into CAS via cas serve HTTP API,
 * subscribes to SSE for activity/inbox events, and posts updates to Slack threads.
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
import { ThreadOwnershipTracker } from "./user-filter.js";
import {
  subscribeToSession,
  type SseEvent,
  type SessionSubscription,
} from "./session-adapter.js";
import {
  formatActivityEvent,
  formatInboxMessage,
  formatError,
  formatInjectionConfirm,
  type CasActivityEvent,
  type CasInboxNotification,
} from "./message-formatter.js";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/** Maps session name → SSE subscription (one per active CAS session). */
const activeSubscriptions = new Map<string, SessionSubscription>();

/** Maps `slack-{thread_ts}` from-label → { channel, thread_ts, session } for routing responses. */
const threadRouting = new Map<
  string,
  { channel: string; thread_ts: string; session: string }
>();

/** Maps session name → Set of from-labels for threads listening to that session. */
const sessionThreads = new Map<string, Set<string>>();

const threadTracker = new ThreadOwnershipTracker();

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main(): Promise<void> {
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

  // --- SSE event handler (scoped to a specific session) ---
  function handleSseEvent(sessionName: string, event: SseEvent): void {
    if (event.event === "activity") {
      try {
        const data = JSON.parse(event.data) as {
          activity: CasActivityEvent[];
        };
        for (const ev of data.activity) {
          const formatted = formatActivityEvent(ev);
          if (!formatted) continue;

          // Only post to threads associated with THIS session
          const labels = sessionThreads.get(sessionName);
          if (!labels) continue;
          for (const fromLabel of labels) {
            const route = threadRouting.get(fromLabel);
            if (!route) continue;
            slack.chat
              .postMessage({
                channel: route.channel,
                thread_ts: route.thread_ts,
                text: formatted.text,
                blocks: formatted.blocks,
              })
              .catch((err) =>
                console.error(`Failed to post activity to Slack: ${err}`),
              );
          }
        }
      } catch {
        // ignore parse errors
      }
    }

    if (event.event === "inbox") {
      try {
        const data = JSON.parse(event.data) as {
          notifications: CasInboxNotification[];
        };
        for (const notif of data.notifications) {
          // Route response back to the originating Slack thread
          const route = threadRouting.get(notif.to);
          if (!route) continue;

          const formatted = formatInboxMessage(notif);
          slack.chat
            .postMessage({
              channel: route.channel,
              thread_ts: route.thread_ts,
              text: formatted.text,
              blocks: formatted.blocks,
            })
            .catch((err) =>
              console.error(`Failed to post inbox reply to Slack: ${err}`),
            );
        }
      } catch {
        // ignore parse errors
      }
    }

    if (event.event === "error") {
      console.error(`SSE error event: ${event.data}`);
    }
  }

  // --- Ensure SSE subscription exists for a session ---
  function ensureSubscription(sessionName: string): void {
    if (activeSubscriptions.has(sessionName)) return;

    console.log(`Subscribing to SSE for session ${sessionName}`);
    const sub = subscribeToSession(
      config.cas_serve_url,
      sessionName,
      config.cas_serve_token,
      0,
      (event) => handleSseEvent(sessionName, event),
      (err) => console.error(`SSE error for ${sessionName}: ${err.message}`),
    );
    activeSubscriptions.set(sessionName, sub);
  }

  // --- Message handler ---
  const handleMessage: MessageHandler = async (msg: DaemonMessage) => {
    console.log(
      `[${msg.project}] Message from ${msg.slack_user}: ${msg.text.slice(0, 80)}`,
    );

    // Thread ownership: register owner or check
    const existingOwner = threadTracker.getOwner(msg.thread_ts);
    if (existingOwner && existingOwner !== msg.slack_user) {
      // Non-owner — reject politely
      await slack.chat.postMessage({
        channel: msg.channel,
        thread_ts: msg.thread_ts,
        text: `This is <@${existingOwner}>'s CAS session — your messages won't be routed to CAS.`,
      });
      return;
    }
    threadTracker.registerOwner(msg.thread_ts, msg.slack_user);

    // Post a "thinking" indicator
    await slack.chat.postMessage({
      channel: msg.channel,
      thread_ts: msg.thread_ts,
      text: `:hourglass_flowing_sand: Processing...`,
    });

    // Execute Claude directly and get the response
    const result = await injectMessage(config, msg);

    if (result.ok && result.response) {
      // Post Claude's response directly to the thread
      // Split long responses into chunks (Slack limit is ~4000 chars per message)
      const maxLen = 3900;
      const response = result.response;
      for (let i = 0; i < response.length; i += maxLen) {
        const chunk = response.slice(i, i + maxLen);
        await slack.chat.postMessage({
          channel: msg.channel,
          thread_ts: msg.thread_ts,
          text: chunk,
        });
      }
    } else {
      const err = formatError("inject_failed", result.error ?? "Unknown error");
      await slack.chat.postMessage({
        channel: msg.channel,
        thread_ts: msg.thread_ts,
        text: err.text,
        blocks: err.blocks,
      });
    }
  };

  const server = startSocketServer(config.socket_path, handleMessage);

  // Graceful shutdown
  const shutdown = () => {
    console.log("Shutting down...");
    for (const [name, sub] of activeSubscriptions) {
      console.log(`Closing SSE for ${name}`);
      sub.abort();
    }
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
