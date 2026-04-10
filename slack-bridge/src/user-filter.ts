/**
 * Thread ownership tracker.
 *
 * Each Slack thread maps to exactly one CAS session owner.
 * Only the owner's messages are forwarded to CAS.
 * Non-owners get a polite rejection notice.
 */

export class ThreadOwnershipTracker {
  /** thread_ts → Slack user ID */
  private owners = new Map<string, string>();
  private maxSize: number;

  constructor(maxSize = 10_000) {
    this.maxSize = maxSize;
  }

  /** Register the owner of a thread. First caller wins — subsequent calls are ignored. */
  registerOwner(threadTs: string, slackUserId: string): void {
    if (this.owners.has(threadTs)) return;

    // Evict oldest if over capacity
    if (this.owners.size >= this.maxSize) {
      const oldest = this.owners.keys().next().value!;
      this.owners.delete(oldest);
    }

    this.owners.set(threadTs, slackUserId);
  }

  /** Get the owner of a thread, or null if unknown. */
  getOwner(threadTs: string): string | null {
    return this.owners.get(threadTs) ?? null;
  }

  /** Check if a user is the owner of a thread. Returns false for unknown threads. */
  isOwner(threadTs: string, slackUserId: string): boolean {
    const owner = this.owners.get(threadTs);
    return owner === slackUserId;
  }
}
