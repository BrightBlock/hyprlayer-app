import { describe, expect, it, vi } from "vitest";

import { releaseChannels } from "./channelRelease";

type FakeChannel = { onmessage: (ev: unknown) => void };

function fake(handler: (ev: unknown) => void): FakeChannel {
  return { onmessage: handler };
}

// The cast mirrors the real Channel<T> shape closely enough for the one
// property this touches; constructing a real Channel needs Tauri internals.
function release(channels: FakeChannel[]) {
  releaseChannels(channels as never);
}

describe("releaseChannels", () => {
  it("replaces the handler so the captured scope is no longer reachable", () => {
    const captured = vi.fn();
    const channels = [fake(captured)];
    const original = channels[0].onmessage;

    release(channels);

    expect(original).toBe(captured);
    expect(channels).toHaveLength(0);
    expect(captured).not.toHaveBeenCalled();
  });

  it("leaves a handler that is safe to invoke after release", () => {
    const captured = vi.fn();
    const channel = fake(captured);
    release([channel]);

    expect(() => channel.onmessage({ kind: "output", data: "x" })).not.toThrow();
    expect(captured).not.toHaveBeenCalled();
  });

  it("releases every channel, not just the first", () => {
    const handlers = [vi.fn(), vi.fn(), vi.fn()];
    const channels = handlers.map(fake);
    const refs = [...channels];

    release(channels);

    for (const [i, ref] of refs.entries()) {
      ref.onmessage({ kind: "output", data: "x" });
      expect(handlers[i]).not.toHaveBeenCalled();
    }
  });

  it("empties the list so a released channel is not retained by it either", () => {
    const channels = [fake(vi.fn()), fake(vi.fn())];
    release(channels);
    expect(channels).toEqual([]);
  });

  it("is a no-op on an empty list", () => {
    const channels: FakeChannel[] = [];
    expect(() => release(channels)).not.toThrow();
    expect(channels).toHaveLength(0);
  });

  it("is idempotent", () => {
    const captured = vi.fn();
    const channels = [fake(captured)];
    release(channels);
    expect(() => release(channels)).not.toThrow();
    expect(captured).not.toHaveBeenCalled();
  });
});
