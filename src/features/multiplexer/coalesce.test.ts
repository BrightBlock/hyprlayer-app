import { describe, expect, it, vi } from "vitest";

import { StateCoalescer } from "./coalesce";
import type { AgentState } from "./engineState";

describe("StateCoalescer", () => {
  it("coalesces multiple flips into one flush per tick (last-write-wins)", () => {
    const ticks: Array<() => void> = [];
    const flush = vi.fn<(b: Map<string, AgentState>) => void>();
    const c = new StateCoalescer(flush, (cb) => ticks.push(cb));

    c.queue("ws:1", "working");
    c.queue("ws:1", "blocked");
    c.queue("ws:1", "working");
    c.queue("ws:1", "done");

    // Nothing committed until the frame runs — no mid-frame repaints.
    expect(flush).not.toHaveBeenCalled();
    expect(ticks).toHaveLength(1);

    ticks[0]();

    expect(flush).toHaveBeenCalledTimes(1);
    const batch = flush.mock.calls[0][0];
    expect(batch.get("ws:1")).toBe("done");
    expect(batch.size).toBe(1);
  });

  it("batches independent agents together in a single frame", () => {
    const ticks: Array<() => void> = [];
    const flush = vi.fn<(b: Map<string, AgentState>) => void>();
    const c = new StateCoalescer(flush, (cb) => ticks.push(cb));

    c.queue("ws:1", "working");
    c.queue("ws:2", "blocked");
    ticks[0]();

    const batch = flush.mock.calls[0][0];
    expect(batch.get("ws:1")).toBe("working");
    expect(batch.get("ws:2")).toBe("blocked");
  });

  it("schedules a fresh frame after the previous one drains", () => {
    const ticks: Array<() => void> = [];
    const flush = vi.fn<(b: Map<string, AgentState>) => void>();
    const c = new StateCoalescer(flush, (cb) => ticks.push(cb));

    c.queue("ws:1", "working");
    ticks[0]();
    c.queue("ws:1", "done");
    expect(ticks).toHaveLength(2);
    ticks[1]();

    expect(flush).toHaveBeenCalledTimes(2);
    expect(flush.mock.calls[1][0].get("ws:1")).toBe("done");
  });
});
