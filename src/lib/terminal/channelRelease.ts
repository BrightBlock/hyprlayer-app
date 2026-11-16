import type { Channel } from "@tauri-apps/api/core";

import { commands } from "@/lib/bindings";

// Tauri deregisters a channel's callback only when the Rust side drops its end,
// which for a terminal is when the pty exits — not when the React component
// unmounts. Until then the handler stays reachable from Tauri's callback
// registry, and it closes over the whole effect scope: the engine, the host
// element, and through that the detached DOM and its WebGL canvases. Replacing
// the handler is what releases that scope.
//
// Measured over 8 dock open/close cycles on a dev build: ~20.5 MB retained per
// remount with no cleanup, ~16.9 MB with the engine dropping its own Terminal
// and FitAddon, ~1.3 MB with this as well.
//
// The server stream behind each channel would otherwise keep pumping over IPC until its terminal exits.
export function releaseChannels<T>(channels: Channel<T>[]): void {
  const ids = channels.map((channel) => channel.id);
  for (const channel of channels) {
    channel.onmessage = () => {};
  }
  channels.length = 0;
  // Fire-and-forget: a gone server (or a test's mocked `commands`) must not throw from a cleanup.
  if (ids.length > 0) void Promise.resolve().then(() => commands.terminalRelease(ids)).catch(() => {});
}
