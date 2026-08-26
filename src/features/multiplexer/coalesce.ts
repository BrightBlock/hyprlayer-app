
import type { AgentState } from "./engineState";

type Scheduler = (cb: () => void) => void;

const raf: Scheduler =
  typeof requestAnimationFrame === "function"
    ? (cb) => requestAnimationFrame(cb)
    : (cb) => setTimeout(cb, 16);

export class StateCoalescer {
  private pending = new Map<string, AgentState>();
  private scheduled = false;

  constructor(
    private readonly flush: (batch: Map<string, AgentState>) => void,
    private readonly schedule: Scheduler = raf,
  ) {}

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
