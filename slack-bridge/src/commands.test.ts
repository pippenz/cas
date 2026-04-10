import { describe, it, expect } from "vitest";
import { detectCommand } from "./commands.js";

describe("detectCommand", () => {
  it("detects shutdown commands", () => {
    expect(detectCommand("shut down")).toBe("shutdown");
    expect(detectCommand("Shut Down")).toBe("shutdown");
    expect(detectCommand("stop")).toBe("shutdown");
    expect(detectCommand("done")).toBe("shutdown");
    expect(detectCommand("kill session")).toBe("shutdown");
    expect(detectCommand("end session")).toBe("shutdown");
  });

  it("detects reset commands", () => {
    expect(detectCommand("reset")).toBe("reset");
    expect(detectCommand("Reset")).toBe("reset");
    expect(detectCommand("start fresh")).toBe("reset");
    expect(detectCommand("new session")).toBe("reset");
    expect(detectCommand("restart")).toBe("reset");
  });

  it("detects status commands", () => {
    expect(detectCommand("status")).toBe("status");
    expect(detectCommand("Status")).toBe("status");
    expect(detectCommand("sessions")).toBe("status");
    expect(detectCommand("session")).toBe("status");
    expect(detectCommand("whats running")).toBe("status");
    expect(detectCommand("what's running")).toBe("status");
  });

  it("returns null for regular messages", () => {
    expect(detectCommand("fix the login bug")).toBeNull();
    expect(detectCommand("please reset the password logic")).toBeNull();
    expect(detectCommand("what is the status of the deployment")).toBeNull();
    expect(detectCommand("")).toBeNull();
    expect(detectCommand("hello")).toBeNull();
  });

  it("handles whitespace", () => {
    expect(detectCommand("  status  ")).toBe("status");
    expect(detectCommand("  shut down  ")).toBe("shutdown");
  });
});
