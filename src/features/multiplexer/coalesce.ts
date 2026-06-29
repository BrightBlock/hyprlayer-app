//! Per-frame coalescer for agent state transitions.
//!
//! A PTY can flip working↔blocked↔done several times between paints as chunks
//! land. Committing each one would repaint the tab dot and recompute the sidebar
//! rollup mid-frame (visible thrash). Instead we buffer the latest state per
//! agent and flush once per animation frame — last-write-wins within the frame.

import type { AgentState } from "./engineState";

type Scheduler = (cb: () => void) => void;

const raf: Scheduler =
  typeof requestAnimationFrame === "function"
    ? (cb) => requestAnimationFrame(cb)
    : (cb) => setTimeout(cb, 16);

export class StateCoalescer {
  private pending = new Map<string, AgentState>();
  private scheduled = false;

  /**
   * @param flush     called once per frame with the batch of changed agents.
   * @param schedule  frame scheduler; injectable so tests drive it manually.
   */
  constructor(
    private readonly flush: (batch: Map<string, AgentState>) => void,
    private readonly schedule: Scheduler = raf,
  ) {}

  /** Queue `state` for `key`; supersedes any state queued for it this frame. */
  queue(key: string, state: AgentState): void {
    this.pending.set(key, state);
    if (!this.scheduled) {
      this.scheduled = true;
      this.schedule(() => this.run());
    }
  }

  private run(): void {
    this.scheduled = false;
    if (this.pending.size === 0) return;
    const batch = this.pending;
    this.pending = new Map();
    this.flush(batch);
  }
}
