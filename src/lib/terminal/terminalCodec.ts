
export const b64ToBytes = (s: string): Uint8Array =>
  Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

export const strToB64 = (s: string): string => {
  const bytes = new TextEncoder().encode(s);
  // Chunking avoids the argument limit on large pastes.
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
};
