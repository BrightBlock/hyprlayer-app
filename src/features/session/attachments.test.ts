// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

const b = vi.hoisted(() => ({
  connectionStatus: vi.fn(),
  saveAttachment: vi.fn(),
  readAttachment: vi.fn(),
  saveClipboardImage: vi.fn(),
}));
const readImage = vi.hoisted(() => vi.fn());
vi.mock("@/lib/bindings", () => ({ commands: b }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ readImage }));
vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: (p: string) => `asset://${p}` }));

import { b64ToBytes, bytesToB64 } from "@/lib/terminal/terminalCodec";
import { saveClipboardImage, saveClipboardImageFromOs, useAttachmentSrc } from "./attachments";

const MIB = 1024 * 1024;
const BLOCK_BUDGET_MS = 50;

function randomBytes(n: number): Uint8Array {
  const bytes = new Uint8Array(n);
  for (let i = 0; i < n; i += 65536) crypto.getRandomValues(bytes.subarray(i, i + 65536));
  return bytes;
}

function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

/** Runs `op` while a 0 ms timer ticks; the widest gap between ticks is the longest block. */
async function longestBlockMs<T>(op: () => Promise<T>): Promise<{ value: T; ms: number }> {
  let longest = 0;
  let last = performance.now();
  let running = true;
  const tick = () => {
    const now = performance.now();
    longest = Math.max(longest, now - last);
    last = now;
    if (running) setTimeout(tick, 0);
  };
  setTimeout(tick, 0);
  const value = await op();
  running = false;
  longest = Math.max(longest, performance.now() - last);
  return { value, ms: longest };
}

afterEach(() => vi.clearAllMocks());

describe("large attachments", () => {
  it("pastes a 16 MiB image without blocking the page", async () => {
    const bytes = randomBytes(16 * MIB);
    b.saveAttachment.mockImplementation(async () => ({ status: "ok", data: "/stored/x.png" }));
    const file = new File([bytes], "shot.png", { type: "image/png" });

    const { value, ms } = await longestBlockMs(() => saveClipboardImage("ws", file));

    expect(value).toEqual({ path: "/stored/x.png", name: "shot.png", kind: "image" });
    const sent = b.saveAttachment.mock.calls[0][2] as string;
    expect(sameBytes(b64ToBytes(sent), bytes)).toBe(true);
    expect(ms).toBeLessThan(BLOCK_BUDGET_MS);
  });

  // Older runtimes take the sync fallback, which is slow by design.
  it.skipIf(!("fromBase64" in Uint8Array))(
    "shows a 16 MiB remote attachment without blocking the page",
    async () => {
      const bytes = randomBytes(16 * MIB);
      const blobs: Blob[] = [];
      URL.createObjectURL = (blob: Blob) => {
        blobs.push(blob);
        return "blob:x";
      };
      URL.revokeObjectURL = () => {};
      b.connectionStatus.mockResolvedValue({ status: "ok", data: { host: "nexus" } });
      const client = new QueryClient();
      const wrapper = ({ children }: { children: ReactNode }) =>
        createElement(QueryClientProvider, { client }, children);
      let resolveRead!: (v: unknown) => void;
      b.readAttachment.mockReturnValue(new Promise((r) => (resolveRead = r)));
      const { result } = renderHook(() => useAttachmentSrc("/stored/x.png"), { wrapper });
      await waitFor(() => expect(b.readAttachment).toHaveBeenCalled());

      const data = bytesToB64(bytes);
      const { ms } = await longestBlockMs(async () => {
        resolveRead({ status: "ok", data });
        await waitFor(() => expect(result.current).toEqual({ src: "blob:x" }));
      });

      const shown = new Uint8Array(await blobs[0].arrayBuffer());
      expect(sameBytes(shown, bytes)).toBe(true);
      expect(ms).toBeLessThan(BLOCK_BUDGET_MS);
    },
  );

  it("saves the OS clipboard image without its pixels entering the page", async () => {
    b.saveClipboardImage.mockResolvedValue({ status: "ok", data: "/stored/pasted-image.png" });

    const att = await saveClipboardImageFromOs("ws");

    expect(att).toEqual({ path: "/stored/pasted-image.png", name: "pasted-image.png", kind: "image" });
    expect(b.saveClipboardImage).toHaveBeenCalledWith("ws", "pasted-image.png");
    expect(readImage).not.toHaveBeenCalled();
  });

  it("adds nothing when the OS clipboard holds no image", async () => {
    b.saveClipboardImage.mockResolvedValue({ status: "ok", data: null });

    expect(await saveClipboardImageFromOs("ws")).toBeNull();
  });
});
