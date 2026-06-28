/** Base64 <-> bytes helpers bridging the PTY `Channel` (which carries base64,
 *  since PTY output is binary) and the xterm engine. Shared by every embedded
 *  terminal view (TerminalTab, ScriptRunView, the onboarding SetupTerminal). */

export const b64ToBytes = (s: string): Uint8Array =>
  Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

export const strToB64 = (s: string): string => {
  const bytes = new TextEncoder().encode(s);
  // Build the binary string in chunks: `String.fromCharCode(...bytes)` spreads
  // every byte as an argument, which overflows the call stack on a large paste.
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
};
