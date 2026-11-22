import { beforeEach, describe, expect, it, vi } from "vitest";

// Ways following the pty's size through the feed could fail:
// - a size event is dropped, or reaches the screen out of order with the output around it;
// - a resume treats its leading size event as the replay and skips the start-over check;
// - a size event moves the offset a reattach resumes from.
// Ways rendering the server's frames could fail:
// - a frame is acked before the screen has it, so the server outruns the webview;
// - a frame resume asks from a byte count, or starts over when the server already did;
// - a program's copy skips the screen's gesture gate.

const acks: Array<[number, number]> = [];
let channels = 0;
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    id = ++channels;
    onmessage: (ev: unknown) => void = () => {};
  },
}));
vi.mock("@/lib/bindings", () => ({
  commands: {
    terminalFrameAck: async (channelId: number, seq: number) => {
      acks.push([channelId, seq]);
      return { status: "ok", data: null };
    },
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
  const pending: Array<() => void> = [];
  const text = (data: Uint8Array | string) =>
    typeof data === "string" ? data : new TextDecoder().decode(data);
  return {
    calls,
    draw: () => pending.splice(0).forEach((written) => written()),
    write: (data: Uint8Array | string) => calls.push(`write:${text(data)}`),
    writeFrame: (data: Uint8Array, written: () => void) => {
      calls.push(`frame:${text(data)}`);
      pending.push(written);
    },
    follow: (g: { cols: number; rows: number }) => calls.push(`follow:${g.cols}x${g.rows}`),
    clipboard: (data: string) => calls.push(`copy:${atob(data)}`),
  };
}

type Send = (ev: unknown) => void;
const sender = (channel: unknown): Send => (ev) =>
  (channel as { onmessage: Send }).onmessage(ev);
const frame = (seq: number, history: number, text: string) => ({
  kind: "output",
  data: b64(text),
  offset: history,
  frame: seq,
});

beforeEach(() => {
  acks.length = 0;
});

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

describe("TerminalFeed frames", () => {
  it("acks a frame only once the screen has drawn it", async () => {
    const s = screen();
    const channel = new TerminalFeed(s, () => {}).channel();
    sender(channel)(frame(1, 0, "screen"));
    await Promise.resolve();
    expect(acks).toEqual([]);

    s.draw();
    await Promise.resolve();
    expect(acks).toEqual([[channel.id, 1]]);
  });

  it("resumes from the history count and leaves starting over to the server", () => {
    const s = screen();
    const feed = new TerminalFeed(s, () => {});
    const send = sender(feed.channel());
    send(frame(1, 0, "first"));
    send(frame(2, 40, "scrolled"));
    const { from, channel } = feed.resume();
    sender(channel)(frame(1, 900, `${START_OVER}fresh`));

    expect(from).toBe(40);
    expect(s.calls).toEqual(["frame:first", "frame:scrolled", `frame:${START_OVER}fresh`]);
  });

  it("hands a program's copy to the screen", () => {
    const s = screen();
    sender(new TerminalFeed(s, () => {}).channel())({ kind: "clipboard", data: b64("copied") });
    expect(s.calls).toEqual(["copy:copied"]);
  });
});
