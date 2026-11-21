import { describe, expect, it, vi } from "vitest";

// Ways following the pty's size through the feed could fail:
// - a size event is dropped, or reaches the screen out of order with the output around it;
// - a resume treats its leading size event as the replay and skips the start-over check;
// - a size event moves the offset a reattach resumes from.

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (ev: unknown) => void = () => {};
  },
}));
vi.mock("./channelRelease", () => ({ releaseChannels: () => {} }));

const { TerminalFeed } = await import("./terminalFeed");

const START_OVER = "\x1bc";

function b64(text: string): string {
  return btoa(text);
}

function screen() {
  const calls: string[] = [];
  return {
    calls,
    write: (data: Uint8Array | string) =>
      calls.push(`write:${typeof data === "string" ? data : new TextDecoder().decode(data)}`),
    follow: (g: { cols: number; rows: number }) => calls.push(`follow:${g.cols}x${g.rows}`),
  };
}

type Send = (ev: unknown) => void;
const sender = (channel: unknown): Send => (ev) =>
  (channel as { onmessage: Send }).onmessage(ev);

describe("TerminalFeed sizes", () => {
  it("hands each size to the screen in order with the output", () => {
    const s = screen();
    const send = sender(new TerminalFeed(s, () => {}).channel());
    send({ kind: "resized", cols: 120, rows: 40 });
    send({ kind: "output", data: b64("old"), offset: 0 });
    send({ kind: "resized", cols: 80, rows: 24 });
    send({ kind: "output", data: b64("new"), offset: 3 });
    expect(s.calls).toEqual(["follow:120x40", "write:old", "follow:80x24", "write:new"]);
  });

  it("does not take a resume's leading size for its replay", () => {
    const s = screen();
    const feed = new TerminalFeed(s, () => {});
    sender(feed.channel())({ kind: "output", data: b64("abc"), offset: 0 });
    const { from, channel } = feed.resume();
    expect(from).toBe(3);
    const send = sender(channel);
    send({ kind: "resized", cols: 100, rows: 30 });
    send({ kind: "output", data: b64("d"), offset: 3 });
    expect(s.calls).toEqual(["write:abc", "follow:100x30", "write:d"]);
  });

  it("still starts over when a resume's first output is not where it asked", () => {
    const s = screen();
    const feed = new TerminalFeed(s, () => {});
    sender(feed.channel())({ kind: "output", data: b64("abc"), offset: 0 });
    const send = sender(feed.resume().channel);
    send({ kind: "resized", cols: 100, rows: 30 });
    send({ kind: "output", data: b64("xyz"), offset: 50 });
    expect(s.calls).toEqual(["write:abc", "follow:100x30", `write:${START_OVER}`, "write:xyz"]);
  });

  it("resumes from the last output, whatever sizes came after it", () => {
    const s = screen();
    const feed = new TerminalFeed(s, () => {});
    const send = sender(feed.channel());
    send({ kind: "output", data: b64("abcd"), offset: 10 });
    send({ kind: "resized", cols: 90, rows: 20 });
    expect(feed.resume().from).toBe(14);
  });
});
