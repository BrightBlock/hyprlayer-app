import { Channel } from "@tauri-apps/api/core";

import type { TerminalEvent } from "@/lib/bindings";
import { releaseChannels } from "./channelRelease";
import { b64ToBytes } from "./terminalCodec";

/** A full reset (RIS) that waits behind the output already queued, which `Terminal.reset()` does not. */
const START_OVER = "\x1bc";

type Screen = {
  write(data: Uint8Array | string): void;
  follow(grid: { cols: number; rows: number }): void;
};

/** One view's terminal output across the streams it opens, so a reattach asks only for what follows. */
export class TerminalFeed {
  /** Where the view's output ends; unset until a server that reports offsets sends some. */
  private next: number | undefined;
  private channels: Channel<TerminalEvent>[] = [];

  constructor(
    private screen: Screen,
    private onExited: () => void,
    private onOutput?: () => void,
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
      if (ev.kind === "exited") return this.onExited();
      if (ev.kind === "resized") return this.screen.follow(ev);
      const bytes = b64ToBytes(ev.data);
      const at = ev.offset ?? undefined;
      // Only a resume's first output can be a replay; a live stream's gap keeps writing, as it always has.
      if (first && from !== undefined && at !== from) this.screen.write(START_OVER);
      first = false;
      this.next = at === undefined ? undefined : at + bytes.length;
      this.screen.write(bytes);
      this.onOutput?.();
    };
    this.channels.push(channel);
    return channel;
  }
}
