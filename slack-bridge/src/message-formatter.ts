/**
 * Format CAS events and messages into Slack-readable output.
 *
 * Converts CAS activity events (from SSE) and inbox messages into
 * Slack message payloads with blocks for rich formatting.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Minimal CAS activity event shape (from SSE `event: activity` data). */
export interface CasActivityEvent {
  id: number;
  event_type: string;
  entity_type: string;
  entity_id: string;
  summary: string;
  metadata: Record<string, unknown> | null;
  created_at: string;
}

/** Minimal CAS inbox notification shape (from SSE `event: inbox` data). */
export interface CasInboxNotification {
  id: number;
  from: string;
  to: string;
  payload: string;
  created_at: string;
}

/** Slack message payload. */
export interface SlackMessage {
  text: string;
  blocks?: SlackBlock[];
}

interface SlackBlock {
  type: string;
  text?: { type: string; text: string };
  elements?: Array<{ type: string; text: string }>;
}

// ---------------------------------------------------------------------------
// ANSI stripping
// ---------------------------------------------------------------------------

const ANSI_RE = /\x1b\[[0-9;]*[a-zA-Z]/g;

function stripAnsi(s: string): string {
  return s.replace(ANSI_RE, "");
}

// ---------------------------------------------------------------------------
// Event types to suppress (internal noise)
// ---------------------------------------------------------------------------

const SUPPRESSED_EVENT_TYPES = new Set([
  "supervisor_injected", // injection confirmations — already confirmed via message_id
  "heartbeat",
  "agent_registered",
  "agent_heartbeat",
]);

// ---------------------------------------------------------------------------
// Formatters
// ---------------------------------------------------------------------------

/**
 * Format a CAS activity event for Slack. Returns null if the event should be suppressed.
 */
export function formatActivityEvent(
  event: CasActivityEvent,
): SlackMessage | null {
  if (SUPPRESSED_EVENT_TYPES.has(event.event_type)) {
    return null;
  }

  const summary = stripAnsi(event.summary);
  const ts = new Date(event.created_at);
  const timeStr = ts.toLocaleTimeString("en-US", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });

  const text = `[${timeStr}] ${summary}`;

  return {
    text,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `\`${timeStr}\` ${summary}`,
        },
      },
    ],
  };
}

/**
 * Format a CAS inbox notification (supervisor/worker response) for Slack.
 */
export function formatInboxMessage(
  notification: CasInboxNotification,
): SlackMessage {
  const payload = stripAnsi(notification.payload);
  const from = notification.from;

  const text = `*${from}:* ${payload}`;

  return {
    text,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `*${from}:*\n>${payload.split("\n").join("\n>")}`,
        },
      },
      {
        type: "context",
        elements: [
          {
            type: "mrkdwn",
            text: `_CAS response at ${new Date(notification.created_at).toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit", hour12: false })}_`,
          },
        ],
      },
    ],
  };
}

/**
 * Format an error for Slack.
 */
export function formatError(code: string, message: string): SlackMessage {
  const text = `Error \`${code}\`: ${message}`;
  return {
    text,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `:warning: *Error* \`${code}\`\n${message}`,
        },
      },
    ],
  };
}

/**
 * Format a message injection confirmation for Slack.
 */
export function formatInjectionConfirm(
  sessionName: string,
  messageId: number,
): SlackMessage {
  const text = `Sent to CAS session \`${sessionName}\` (message #${messageId})`;
  return {
    text,
    blocks: [
      {
        type: "context",
        elements: [
          {
            type: "mrkdwn",
            text: `Sent to CAS session \`${sessionName}\` (message #${messageId})`,
          },
        ],
      },
    ],
  };
}
