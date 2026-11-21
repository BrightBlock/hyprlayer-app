import { commands } from "@/lib/bindings";

import { strToB64 } from "./terminalCodec";

// Tauri and the server run each request on its own task, so two writes in flight can
// land out of order: xterm's DA1 reply overtaking its OSC 11 reply reads as no answer.
const queued = new Map<string, string>();

/** Sends input to a pty in order: one write in flight, anything typed meanwhile batched behind it. */
export function writeTerminal(terminalId: string, data: string): void {
  const pending = queued.get(terminalId);
  if (pending !== undefined) {
    queued.set(terminalId, pending + data);
    return;
  }
  queued.set(terminalId, "");
  void drain(terminalId, data);
}

/** A terminal opened in the focused window takes this desktop's size from any other sharing it. */
export function claimIfFocused(terminalId: string): void {
  if (document.hasFocus()) void commands.terminalFocus(terminalId);
}

/** A press in a terminal takes its size back from another desktop sharing it; returns the unsubscribe. */
export function claimOnPress(host: HTMLElement, terminalId: () => string | undefined): () => void {
  const claim = () => {
    const id = terminalId();
    if (id) void commands.terminalFocus(id);
  };
  // Capture: xterm keeps a press to itself while the program tracks the mouse.
  host.addEventListener("mousedown", claim, true);
  return () => host.removeEventListener("mousedown", claim, true);
}

async function drain(terminalId: string, data: string): Promise<void> {
  for (let next = data; next; ) {
    await commands.terminalWrite(terminalId, strToB64(next)).catch(() => undefined);
    next = queued.get(terminalId) ?? "";
    queued.set(terminalId, "");
  }
  queued.delete(terminalId);
}
