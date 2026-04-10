import { describe, it, expect, beforeEach } from "vitest";
import { ThreadOwnershipTracker } from "../user-filter.js";

describe("ThreadOwnershipTracker", () => {
  let tracker: ThreadOwnershipTracker;

  beforeEach(() => {
    tracker = new ThreadOwnershipTracker();
  });

  it("registers thread owner on first message", () => {
    tracker.registerOwner("thread-1", "U_BEN");
    expect(tracker.getOwner("thread-1")).toBe("U_BEN");
  });

  it("allows messages from thread owner", () => {
    tracker.registerOwner("thread-1", "U_BEN");
    expect(tracker.isOwner("thread-1", "U_BEN")).toBe(true);
  });

  it("rejects messages from non-owner", () => {
    tracker.registerOwner("thread-1", "U_BEN");
    expect(tracker.isOwner("thread-1", "U_DANIEL")).toBe(false);
  });

  it("returns null for unknown threads", () => {
    expect(tracker.getOwner("unknown-thread")).toBeNull();
  });

  it("treats unknown thread + isOwner as false", () => {
    expect(tracker.isOwner("unknown-thread", "U_BEN")).toBe(false);
  });

  it("does not overwrite existing owner", () => {
    tracker.registerOwner("thread-1", "U_BEN");
    tracker.registerOwner("thread-1", "U_DANIEL");
    expect(tracker.getOwner("thread-1")).toBe("U_BEN");
  });

  it("tracks multiple threads independently", () => {
    tracker.registerOwner("thread-1", "U_BEN");
    tracker.registerOwner("thread-2", "U_DANIEL");
    expect(tracker.isOwner("thread-1", "U_BEN")).toBe(true);
    expect(tracker.isOwner("thread-2", "U_DANIEL")).toBe(true);
    expect(tracker.isOwner("thread-1", "U_DANIEL")).toBe(false);
  });

  it("evicts oldest entries when over capacity", () => {
    const small = new ThreadOwnershipTracker(3);
    small.registerOwner("t1", "U1");
    small.registerOwner("t2", "U2");
    small.registerOwner("t3", "U3");
    small.registerOwner("t4", "U4"); // should evict t1
    expect(small.getOwner("t1")).toBeNull();
    expect(small.getOwner("t4")).toBe("U4");
  });
});
