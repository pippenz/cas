/**
 * Configuration for the CAS Slack bridge.
 *
 * The router reads this to map Slack channels → projects and Slack users → Linux users.
 * Per-user daemons read their own section to know which cas serve endpoint to hit.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface ChannelConfig {
  /** CAS project name (used for session lookup) */
  project: string;
  /** Directory name under the user's project root (e.g., "gabber-studio") */
  dir_name: string;
}

export interface BridgeConfig {
  /** Slack channel ID → project mapping */
  channels: Record<string, ChannelConfig>;

  /** Slack user ID → Linux username allowlist */
  users: Record<string, string>;

  /** Directory containing per-user Unix sockets (default: /run/cas-bridge) */
  socket_dir: string;

  /** Base directory where users keep their projects (default: /home/{user}) */
  projects_base: string;
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

const DEFAULT_CONFIG: BridgeConfig = {
  channels: {},
  users: {},
  socket_dir: "/run/cas-bridge",
  projects_base: "/home",
};

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/**
 * Load config from a JSON file, falling back to defaults for missing fields.
 * Config path: CAS_BRIDGE_CONFIG env var, or /etc/cas-bridge/config.json.
 */
export function loadConfig(path?: string): BridgeConfig {
  const configPath =
    path ?? process.env.CAS_BRIDGE_CONFIG ?? "/etc/cas-bridge/config.json";

  let raw: Record<string, unknown> = {};
  try {
    const content = readFileSync(resolve(configPath), "utf-8");
    raw = JSON.parse(content) as Record<string, unknown>;
  } catch (err) {
    const code = (err as NodeJS.ErrnoException).code;
    if (code === "ENOENT") {
      console.warn(`Config not found at ${configPath}, using defaults`);
    } else {
      throw err;
    }
  }

  return {
    ...DEFAULT_CONFIG,
    ...raw,
    channels: (raw.channels as Record<string, ChannelConfig>) ?? {},
    users: (raw.users as Record<string, string>) ?? {},
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Resolve the Linux username for a Slack user ID, or null if not allowlisted. */
export function resolveUser(
  config: BridgeConfig,
  slackUserId: string,
): string | null {
  return config.users[slackUserId] ?? null;
}

/** Resolve the project config for a Slack channel ID, or null if unmapped. */
export function resolveChannel(
  config: BridgeConfig,
  channelId: string,
): ChannelConfig | null {
  return config.channels[channelId] ?? null;
}

/** Get the Unix socket path for a given Linux username. */
export function socketPathForUser(
  config: BridgeConfig,
  username: string,
): string {
  return `${config.socket_dir}/${username}.sock`;
}

/** Get the project directory for a user + channel combination. */
export function projectDirForUser(
  config: BridgeConfig,
  username: string,
  channel: ChannelConfig,
): string {
  return `${config.projects_base}/${username}/${channel.dir_name}`;
}
