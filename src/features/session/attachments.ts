
import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

import { commands } from "@/lib/bindings";
import { errorMessage, unwrap } from "@/lib/ipc";
import { useConnectionStatus } from "@/lib/queries";
import { b64ToBytes } from "@/lib/terminal/terminalCodec";
import { baseName } from "./toolDisplay";

export type AttachmentKind = "image" | "file";

export type Attachment = {
  path: string;
  name: string;
  kind: AttachmentKind;
};

export const IMAGE_EXTENSIONS = [
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "bmp",
  "svg",
  "heic",
  "heif",
  "tiff",
  "tif",
  "avif",
];

const IMAGE_EXT = new Set(IMAGE_EXTENSIONS);

function extOf(path: string): string {
  return path.split(".").pop()?.toLowerCase() ?? "";
}

export function isImagePath(path: string): boolean {
  return IMAGE_EXT.has(extOf(path));
}

/** A blob carries its own Content-Type, and `<img>` never sniffs SVG. */
const MIME_FOR_EXT: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  bmp: "image/bmp",
  svg: "image/svg+xml",
  heic: "image/heic",
  heif: "image/heif",
  tiff: "image/tiff",
  tif: "image/tiff",
  avif: "image/avif",
};

// Native where the webview has it: the JS fallback blocks ~120 ms per 16 MiB in WebKit.
function decodeB64(data: string): Uint8Array {
  const u8 = Uint8Array as { fromBase64?: (s: string) => Uint8Array };
  return u8.fromBase64 ? u8.fromBase64(data) : b64ToBytes(data);
}

/**
 * The image `src` for a stored attachment, resolved on the machine the
 * bytes live on. Local server: `convertFileSrc` synchronously — no copy,
 * nothing held in memory. Remote server: the path means nothing to this
 * webview's filesystem, so the bytes are fetched over RPC and turned into a
 * revocable `blob:` URL.
 *
 * Reports a failure rather than swallowing it: an oversized or refused
 * attachment used to leave the card on its loading fallback forever, which
 * looks identical to a slow fetch and says nothing about why.
 */
export function useAttachmentSrc(path: string): { src?: string; error?: string } {
  const status = useConnectionStatus();
  const remote = status.data?.host != null;
  const [state, setState] = useState<{ src?: string; error?: string }>({});

  useEffect(() => {
    if (!remote) return;
    let cancelled = false;
    let url: string | undefined;
    setState({});
    unwrap(commands.readAttachment(path))
      .then((data) => {
        if (cancelled) return;
        const type = MIME_FOR_EXT[extOf(path)] ?? "";
        url = URL.createObjectURL(new Blob([decodeB64(data)], { type }));
        setState({ src: url });
      })
      .catch((e: unknown) => {
        if (!cancelled) setState({ error: errorMessage(e) });
      });
    return () => {
      cancelled = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [remote, path]);

  return remote ? state : { src: convertFileSrc(path) };
}

/**
 * Whether the app can open this attachment with the OS. A remote server's path
 * does not exist on this machine, so `openPath` there resolves nothing and
 * fails silently — better to say so than to offer a button that does nothing.
 */
export function useCanOpenAttachment(): boolean {
  return useConnectionStatus().data?.host == null;
}

const EXT_FOR_MIME: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/bmp": "bmp",
  "image/svg+xml": "svg",
  "image/tiff": "tiff",
  "image/avif": "avif",
};

export function clipboardImageName(file: File): string {
  if (file.name && file.name !== "image.png") return file.name;
  const ext = EXT_FOR_MIME[file.type] ?? "png";
  return `pasted-image.${ext}`;
}

// FileReader encodes natively without blocking; a JS loop blocks ~130 ms per 16 MiB in WebKit.
function blobToB64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = reader.result as string;
      resolve(url.slice(url.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

export async function importDroppedPaths(
  workstreamId: string,
  paths: string[],
): Promise<Attachment[]> {
  const imported = await Promise.all(
    paths.map(async (path): Promise<Attachment | null> => {
      try {
        if (!isImagePath(path)) return { path, name: baseName(path), kind: "file" };
        const stored = await unwrap(commands.importAttachment(workstreamId, path));
        return { path: stored, name: baseName(path), kind: "image" };
      } catch {
        return null;
      }
    }),
  );
  return imported.filter((a): a is Attachment => a !== null);
}

export async function saveClipboardImage(
  workstreamId: string,
  file: File,
): Promise<Attachment | null> {
  try {
    const name = clipboardImageName(file);
    const data = await blobToB64(file);
    const stored = await unwrap(commands.saveAttachment(workstreamId, name, data));
    return { path: stored, name, kind: "image" };
  } catch {
    return null;
  }
}

// Rust reads and encodes the image: its pixels, about 33 MB for a 4K screen, never reach the page.
export async function saveClipboardImageFromOs(
  workstreamId: string,
): Promise<Attachment | null> {
  try {
    const name = "pasted-image.png";
    const stored = await unwrap(commands.saveClipboardImage(workstreamId, name));
    return stored ? { path: stored, name, kind: "image" } : null;
  } catch {
    return null;
  }
}
