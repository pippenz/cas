import { describe, it, expect, beforeEach, vi } from "vitest";
import { SessionManager, type SessionEntry, DEFAULT_SESSION_CONFIG } from "./session-manager.js";
import type { DaemonConfig } from "./daemon.js";

const mockConfig: DaemonConfig = {
  socket_path: "/tmp/test.sock",
  username: "testuser",
  cas_serve_url: "http://127.0.0.1:18999",
  cas_serve_token: "",
  slack_bot_token: "xoxb-test",
};

describe("SessionManager", () => {
  let manager: SessionManager;

  beforeEach(() => {
    manager = new SessionManager(mockConfig, {
      idle_timeout_ms: 10_000,
      idle_warning_ms: 3_000,
      idle_check_interval_ms: 1_000,
    });
  });

  describe("tracking", () => {
    it("starts with no sessions", () => {
      expect(manager.size).toBe(0);
      expect(manager.list()).toEqual([]);
    });

    it("tracks a session", () => {
      const entry: SessionEntry = {
        name: "test-session-1",
        project_dir: "/home/user/project-a",
        last_activity: Date.now(),
        started_at: Date.now(),
      };
      manager.track(entry);
      expect(manager.size).toBe(1);
      expect(manager.get("/home/user/project-a")).toEqual(entry);
    });

    it("untracks a session", () => {
      manager.track({
        name: "test-session-1",
        project_dir: "/home/user/project-a",
        last_activity: Date.now(),
        started_at: Date.now(),
      });
      manager.untrack("/home/user/project-a");
      expect(manager.size).toBe(0);
      expect(manager.get("/home/user/project-a")).toBeNull();
    });

    it("touches a session to update last_activity", () => {
      const oldTime = Date.now() - 60_000;
      manager.track({
        name: "test-session-1",
        project_dir: "/home/user/project-a",
        last_activity: oldTime,
        started_at: oldTime,
      });

      manager.touch("/home/user/project-a");

      const entry = manager.get("/home/user/project-a");
      expect(entry).not.toBeNull();
      expect(entry!.last_activity).toBeGreaterThan(oldTime);
    });

    it("lists all tracked sessions", () => {
      manager.track({
        name: "session-a",
        project_dir: "/home/user/project-a",
        last_activity: Date.now(),
        started_at: Date.now(),
      });
      manager.track({
        name: "session-b",
        project_dir: "/home/user/project-b",
        last_activity: Date.now(),
        started_at: Date.now(),
      });

      expect(manager.list()).toHaveLength(2);
    });

    it("returns null for unknown project", () => {
      expect(manager.get("/nonexistent")).toBeNull();
    });
  });

  describe("idle detection", () => {
    it("detects sessions past the full timeout", async () => {
      // Mock fetch to avoid real HTTP calls (killSession calls cas serve)
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        new Response(JSON.stringify({ ok: true }), { status: 200 }),
      );

      const oldTime = Date.now() - 15_000; // 15s ago, timeout is 10s
      manager.track({
        name: "old-session",
        project_dir: "/home/user/project-a",
        last_activity: oldTime,
        started_at: oldTime,
      });

      const callbacks: Array<{ entry: SessionEntry; warning: boolean }> = [];
      manager.startIdleChecker(async (entry, warning) => {
        callbacks.push({ entry, warning });
      });

      await manager.checkIdle();
      manager.stopIdleChecker();

      expect(callbacks).toHaveLength(1);
      expect(callbacks[0].warning).toBe(false); // full timeout, not warning
      expect(manager.size).toBe(0); // session was removed
      expect(fetchSpy).toHaveBeenCalledOnce(); // killSession was called

      fetchSpy.mockRestore();
    });

    it("sends warning before full timeout", async () => {
      // idle_warning_ms = 3000, idle_timeout_ms = 10000
      // so warning fires at 7000ms idle
      const warningTime = Date.now() - 8_000; // 8s ago, warning at 7s
      manager.track({
        name: "idle-session",
        project_dir: "/home/user/project-a",
        last_activity: warningTime,
        started_at: warningTime,
      });

      const callbacks: Array<{ entry: SessionEntry; warning: boolean }> = [];
      manager.startIdleChecker(async (entry, warning) => {
        callbacks.push({ entry, warning });
      });

      await manager.checkIdle();
      manager.stopIdleChecker();

      expect(callbacks).toHaveLength(1);
      expect(callbacks[0].warning).toBe(true);
      expect(manager.size).toBe(1); // session still alive
    });

    it("does not warn twice", async () => {
      const warningTime = Date.now() - 8_000;
      manager.track({
        name: "idle-session",
        project_dir: "/home/user/project-a",
        last_activity: warningTime,
        started_at: warningTime,
      });

      const callbacks: Array<{ entry: SessionEntry; warning: boolean }> = [];
      manager.startIdleChecker(async (entry, warning) => {
        callbacks.push({ entry, warning });
      });

      await manager.checkIdle();
      await manager.checkIdle(); // second check — should not warn again
      manager.stopIdleChecker();

      expect(callbacks).toHaveLength(1); // only one warning
    });

    it("resets warning when activity happens", async () => {
      const warningTime = Date.now() - 8_000;
      manager.track({
        name: "idle-session",
        project_dir: "/home/user/project-a",
        last_activity: warningTime,
        started_at: warningTime,
      });

      const callbacks: Array<{ entry: SessionEntry; warning: boolean }> = [];
      manager.startIdleChecker(async (entry, warning) => {
        callbacks.push({ entry, warning });
      });

      await manager.checkIdle(); // triggers warning
      manager.touch("/home/user/project-a"); // user activity resets warning
      await manager.checkIdle(); // should not warn again (activity is fresh)
      manager.stopIdleChecker();

      expect(callbacks).toHaveLength(1);
    });
  });

  describe("default config", () => {
    it("has expected defaults", () => {
      expect(DEFAULT_SESSION_CONFIG.idle_timeout_ms).toBe(30 * 60 * 1000);
      expect(DEFAULT_SESSION_CONFIG.idle_warning_ms).toBe(5 * 60 * 1000);
      expect(DEFAULT_SESSION_CONFIG.idle_check_interval_ms).toBe(60 * 1000);
    });
  });
});
