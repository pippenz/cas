/**
 * File upload passthrough for the Slack bridge.
 *
 * Downloads files shared in CAS channels, sanitizes filenames (security),
 * stages them in the project's .cas/uploads/ directory, and injects a
 * notification to the CAS supervisor.
 */

import { existsSync, mkdirSync, writeFileSync, lstatSync, realpathSync } from "node:fs";
import { join, basename, resolve } from "node:path";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** Maximum file size in bytes (10 MB) */
export const MAX_FILE_SIZE = 10 * 1024 * 1024;

/** Allowed filename characters: alphanumeric, dot, dash, underscore */
const SAFE_FILENAME_RE = /^[a-zA-Z0-9._-]+$/;

/** Upload staging subdirectory under the project's .cas/ */
const UPLOADS_DIR = ".cas/uploads";

// ---------------------------------------------------------------------------
// Filename sanitization
// ---------------------------------------------------------------------------

/**
 * Sanitize a filename for safe local storage.
 *
 * Security measures:
 * - Strips path separators (/ and \)
 * - Replaces disallowed characters with underscores
 * - Rejects empty filenames
 * - Limits length to 255 characters
 */
export function sanitizeFilename(raw: string): string | null {
  // Strip path components — only keep the base name
  let name = basename(raw.replace(/\\/g, "/"));

  // Replace any character not in the safe set
  name = name.replace(/[^a-zA-Z0-9._-]/g, "_");

  // Collapse consecutive underscores
  name = name.replace(/_+/g, "_");

  // Trim leading/trailing underscores and dots (prevent hidden files)
  name = name.replace(/^[._]+/, "").replace(/[._]+$/, "");

  // Reject empty or too-short results
  if (!name || name.length < 1) return null;

  // Truncate to 255 chars
  if (name.length > 255) {
    name = name.slice(0, 255);
  }

  return name;
}

/**
 * Validate that a resolved path is a child of the expected parent directory.
 * Prevents path traversal attacks.
 */
export function isChildOf(childPath: string, parentPath: string): boolean {
  const resolvedChild = resolve(childPath);
  const resolvedParent = resolve(parentPath);
  return resolvedChild.startsWith(resolvedParent + "/") || resolvedChild === resolvedParent;
}

// ---------------------------------------------------------------------------
// File staging
// ---------------------------------------------------------------------------

export interface StageResult {
  ok: boolean;
  /** Absolute path to the staged file */
  path?: string;
  /** Sanitized filename */
  filename?: string;
  /** Error message if staging failed */
  error?: string;
}

/**
 * Stage file content into the project's .cas/uploads/ directory.
 *
 * Security checks:
 * - Filename sanitization
 * - Size cap (MAX_FILE_SIZE)
 * - Path traversal prevention (realpath check)
 * - Symlink rejection
 */
export function stageFile(
  projectDir: string,
  rawFilename: string,
  content: Buffer,
): StageResult {
  // Size check
  if (content.length > MAX_FILE_SIZE) {
    const sizeMB = (content.length / (1024 * 1024)).toFixed(1);
    return { ok: false, error: `File too large (${sizeMB}MB). Maximum is 10MB.` };
  }

  // Sanitize filename
  const filename = sanitizeFilename(rawFilename);
  if (!filename) {
    return { ok: false, error: `Invalid filename: "${rawFilename}"` };
  }

  // Ensure uploads directory exists
  const uploadsDir = join(projectDir, UPLOADS_DIR);
  mkdirSync(uploadsDir, { recursive: true });

  // Build target path and verify it's inside the uploads directory
  const targetPath = join(uploadsDir, filename);
  if (!isChildOf(targetPath, uploadsDir)) {
    return { ok: false, error: "Path traversal detected — rejected." };
  }

  // Write the file
  writeFileSync(targetPath, content);

  // Post-write safety: verify the written path is not a symlink
  // and resolves to inside the uploads directory
  try {
    const stat = lstatSync(targetPath);
    if (stat.isSymbolicLink()) {
      // Remove the symlink — this shouldn't happen since we just wrote it,
      // but defense in depth against TOCTOU races
      const { unlinkSync } = require("node:fs") as typeof import("node:fs");
      unlinkSync(targetPath);
      return { ok: false, error: "Symlink detected at target path — rejected." };
    }

    const realPath = realpathSync(targetPath);
    if (!isChildOf(realPath, uploadsDir)) {
      const { unlinkSync } = require("node:fs") as typeof import("node:fs");
      unlinkSync(targetPath);
      return { ok: false, error: "Resolved path escapes uploads directory — rejected." };
    }
  } catch {
    return { ok: false, error: "Failed to verify staged file." };
  }

  return { ok: true, path: targetPath, filename };
}

// ---------------------------------------------------------------------------
// Slack file download
// ---------------------------------------------------------------------------

/**
 * Download a file from Slack's API.
 * Requires the bot token for authorization.
 */
export async function downloadSlackFile(
  fileUrl: string,
  botToken: string,
): Promise<{ ok: boolean; content?: Buffer; error?: string }> {
  try {
    const res = await fetch(fileUrl, {
      headers: { Authorization: `Bearer ${botToken}` },
    });

    if (!res.ok) {
      return { ok: false, error: `Slack download failed: ${res.status}` };
    }

    const arrayBuffer = await res.arrayBuffer();
    const content = Buffer.from(arrayBuffer);

    return { ok: true, content };
  } catch (err) {
    return { ok: false, error: `Download error: ${err}` };
  }
}

// ---------------------------------------------------------------------------
// Supervisor notification message
// ---------------------------------------------------------------------------

/**
 * Build the message to inject into the CAS supervisor about an uploaded file.
 */
export function buildUploadNotification(
  slackUser: string,
  filename: string,
  filePath: string,
  originalFilename: string,
): string {
  return [
    `File uploaded by Slack user <@${slackUser}>:`,
    `  Original: ${originalFilename}`,
    `  Staged at: ${filePath}`,
    ``,
    `The file is available for reading. Use the Read tool to inspect it.`,
  ].join("\n");
}
