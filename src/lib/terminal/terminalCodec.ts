export const b64ToBytes = (s: string): Uint8Array =>
  Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

export const bytesToB64 = (bytes: Uint8Array): string => {
  // Chunking avoids the argument limit on large pastes.
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
};

export const strToB64 = (s: string): string => bytesToB64(new TextEncoder().encode(s));
