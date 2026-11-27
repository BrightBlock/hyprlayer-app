import { Channel } from "@tauri-apps/api/core";

import { commands, type TerminalEvent } from "@/lib/bindings";
import { releaseChannels } from "./channelRelease";
import { b64ToBytes } from "./terminalCodec";
import { terminalFrame } from "../perfProbe";

/** A full reset (RIS) that waits behind the output already queued, which `Terminal.reset()` does not. */
const START_OVER = "\x1bc";

type Output = Extract<TerminalEvent, { kind: "output" }>;

type Screen = {
  write(data: Uint8Array | string): void;
  writeFrame(data: Uint8Array, written: () => void): void;
  follow(grid: { cols: number; rows: number }): void;
  clipboard(data: string): void;
};

/** Where a reattach picks up after `ev`: the end of raw output, or a frame's history count. */
export function resumePoint(ev: Output, length: number): number | undefined {
  if (ev.offset == null) return undefined;
  return ev.frame == null ? ev.offset + length : ev.offset;
}

/** One view's terminal output across the streams it opens, so a reattach asks only for what follows. */
export class TerminalFeed {
  /** Where the view's output ends; unset until a server that reports offsets sends some. */
  next: number | undefined;
  private channels: Channel<TerminalEvent>[] = [];

  constructor(
    private screen: Screen,
    private onExited: (code: number | null) => void,
    private onOutput?: () => void,
    private onOwner?: (owner: boolean) => void,
  ) {}

  /** A stream onto an empty screen: a mount's attach or a fresh terminal. */
  channel(): Channel<TerminalEvent> {
    return this.open(undefined);
  }

  /** A reattach after the old stream died: from the last byte shown, or a replay onto a cleared screen. */
  resume(): { from: number | null; channel: Channel<TerminalEvent> } {
    this.release();
    const from = this.next;
    if (from === undefined) this.screen.write(START_OVER);
    return { from: from ?? null, channel: this.open(from) };
  }

  /** The view moves to another terminal: the old streams go quiet and their offset is forgotten. */
  restart(): void {
    this.release();
    this.next = undefined;
    this.screen.write(START_OVER);
  }

  release(): void {
    releaseChannels(this.channels);
  }

  private open(from: number | undefined): Channel<TerminalEvent> {
    const channel = new Channel<TerminalEvent>();
    let first = true;
    channel.onmessage = (ev) => {
      if (ev.kind === "exited") return this.onExited(ev.code);
      if (ev.kind === "resized") {
        if (ev.owner != null) this.onOwner?.(ev.owner);
        return this.screen.follow(ev);
      }
      if (ev.kind === "clipboard") return this.screen.clipboard(ev.data);
      const bytes = b64ToBytes(ev.data);
      this.next = resumePoint(ev, bytes.length);
      const seq = ev.frame;
      if (seq != null) {
        // The server starts a frame stream it can't resume over itself.
        this.screen.writeFrame(bytes, () => {
          if (import.meta.env.VITE_PERF_PROBE) terminalFrame(bytes);
          void commands.terminalFrameAck(channel.id, seq);
        });
      } else {
        // Only a resume's first output can be a replay; a live stream's gap keeps writing, as it always has.
        if (first && from !== undefined && ev.offset !== from) this.screen.write(START_OVER);
        this.screen.write(bytes);
      }
      first = false;
      this.onOutput?.();
    };
    this.channels.push(channel);
    return channel;
  }
}
