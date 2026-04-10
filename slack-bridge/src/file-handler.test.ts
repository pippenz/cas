import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdirSync, existsSync, readFileSync, symlinkSync, rmSync } from "node:fs";
import { join } from "node:path";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import {
  sanitizeFilename,
  isChildOf,
  stageFile,
  buildUploadNotification,
  MAX_FILE_SIZE,
} from "./file-handler.js";

// ---------------------------------------------------------------------------
// sanitizeFilename
// ---------------------------------------------------------------------------

describe("sanitizeFilename", () => {
  it("passes clean filenames through", () => {
    expect(sanitizeFilename("report.pdf")).toBe("report.pdf");
    expect(sanitizeFilename("my-file_v2.txt")).toBe("my-file_v2.txt");
    expect(sanitizeFilename("IMAGE001.PNG")).toBe("IMAGE001.PNG");
  });

  it("strips path separators (keeps only basename)", () => {
    expect(sanitizeFilename("../../../etc/passwd")).toBe("passwd");
    expect(sanitizeFilename("/tmp/evil.sh")).toBe("evil.sh");
    expect(sanitizeFilename("..\\..\\windows\\system32\\cmd.exe")).toBe("cmd.exe");
  });

  it("replaces disallowed characters", () => {
    expect(sanitizeFilename("file name (1).txt")).toBe("file_name_1_.txt");
    expect(sanitizeFilename("résumé.pdf")).toBe("r_sum_.pdf");
  });

  it("collapses consecutive underscores", () => {
    expect(sanitizeFilename("a   b   c.txt")).toBe("a_b_c.txt");
  });

  it("strips leading dots (hidden files)", () => {
    expect(sanitizeFilename(".hidden")).toBe("hidden");
    expect(sanitizeFilename("..htaccess")).toBe("htaccess");
  });

  it("rejects empty filenames", () => {
    expect(sanitizeFilename("")).toBeNull();
    expect(sanitizeFilename("...")).toBeNull();
    expect(sanitizeFilename("___")).toBeNull();
  });

  it("truncates long filenames", () => {
    const long = "a".repeat(300) + ".txt";
    const result = sanitizeFilename(long);
    expect(result).not.toBeNull();
    expect(result!.length).toBeLessThanOrEqual(255);
  });
});

// ---------------------------------------------------------------------------
// isChildOf
// ---------------------------------------------------------------------------

describe("isChildOf", () => {
  it("accepts child paths", () => {
    expect(isChildOf("/home/user/project/.cas/uploads/file.txt", "/home/user/project/.cas/uploads")).toBe(true);
  });

  it("rejects parent escape", () => {
    expect(isChildOf("/home/user/project/.cas/uploads/../../../etc/passwd", "/home/user/project/.cas/uploads")).toBe(false);
  });

  it("rejects sibling paths", () => {
    expect(isChildOf("/home/user/other/file.txt", "/home/user/project/.cas/uploads")).toBe(false);
  });

  it("accepts exact parent", () => {
    expect(isChildOf("/home/user/project/.cas/uploads", "/home/user/project/.cas/uploads")).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// stageFile
// ---------------------------------------------------------------------------

describe("stageFile", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "cas-upload-test-"));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("stages a valid file", () => {
    const content = Buffer.from("hello world");
    const result = stageFile(tmpDir, "test.txt", content);

    expect(result.ok).toBe(true);
    expect(result.filename).toBe("test.txt");
    expect(result.path).toContain(".cas/uploads/test.txt");
    expect(existsSync(result.path!)).toBe(true);
    expect(readFileSync(result.path!, "utf-8")).toBe("hello world");
  });

  it("creates the uploads directory if missing", () => {
    const uploadsDir = join(tmpDir, ".cas", "uploads");
    expect(existsSync(uploadsDir)).toBe(false);

    const result = stageFile(tmpDir, "new.txt", Buffer.from("data"));
    expect(result.ok).toBe(true);
    expect(existsSync(uploadsDir)).toBe(true);
  });

  it("rejects files over 10MB", () => {
    const bigContent = Buffer.alloc(MAX_FILE_SIZE + 1, "x");
    const result = stageFile(tmpDir, "big.bin", bigContent);

    expect(result.ok).toBe(false);
    expect(result.error).toContain("too large");
  });

  it("sanitizes path traversal in filename to safe basename", () => {
    const content = Buffer.from("safe content");
    const result = stageFile(tmpDir, "../../../etc/passwd", content);

    // basename() strips path components, leaving just "passwd"
    expect(result.ok).toBe(true);
    expect(result.filename).toBe("passwd");
    expect(result.path).toContain(".cas/uploads/passwd");
  });

  it("rejects invalid filenames", () => {
    const result = stageFile(tmpDir, "...", Buffer.from("data"));
    expect(result.ok).toBe(false);
    expect(result.error).toContain("Invalid filename");
  });

  it("rejects symlinks pointing outside uploads dir", () => {
    // Pre-create the uploads dir with a symlink pointing outside
    const uploadsDir = join(tmpDir, ".cas", "uploads");
    mkdirSync(uploadsDir, { recursive: true });

    const outsideTarget = join(tmpDir, "outside.txt");
    const { writeFileSync: wfs } = require("node:fs") as typeof import("node:fs");
    wfs(outsideTarget, "outside data");

    // Create a symlink inside uploads that points outside
    const symlinkPath = join(uploadsDir, "escape.txt");
    symlinkSync(outsideTarget, symlinkPath);

    // writeFileSync follows the symlink, writing to the outside target.
    // Post-write realpathSync check catches that the real path is outside uploads.
    const result = stageFile(tmpDir, "escape.txt", Buffer.from("injected"));
    expect(result.ok).toBe(false);
    expect(result.error).toContain("rejected");
  });

  it("sanitizes filenames with special characters", () => {
    const result = stageFile(tmpDir, "my file (copy).txt", Buffer.from("data"));
    expect(result.ok).toBe(true);
    expect(result.filename).toBe("my_file_copy_.txt");
  });
});

// ---------------------------------------------------------------------------
// buildUploadNotification
// ---------------------------------------------------------------------------

describe("buildUploadNotification", () => {
  it("builds a readable notification", () => {
    const msg = buildUploadNotification(
      "U12345",
      "screenshot.png",
      "/home/user/project/.cas/uploads/screenshot.png",
      "Screenshot 2024-01-01.png",
    );

    expect(msg).toContain("File uploaded by Slack user <@U12345>");
    expect(msg).toContain("Screenshot 2024-01-01.png");
    expect(msg).toContain("/home/user/project/.cas/uploads/screenshot.png");
    expect(msg).toContain("Read tool");
  });
});
