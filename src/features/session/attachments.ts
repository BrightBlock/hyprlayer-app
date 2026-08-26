
import { convertFileSrc } from "@tauri-apps/api/core";

import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
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

export function isImagePath(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return IMAGE_EXT.has(ext);
}

export function attachmentSrc(path: string): string {
  return convertFileSrc(path);
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
    const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
    const stored = await unwrap(commands.saveAttachment(workstreamId, name, bytes));
    return { path: stored, name, kind: "image" };
  } catch {
    return null;
  }
}

export async function saveClipboardImageFromOs(
  workstreamId: string,
): Promise<Attachment | null> {
  try {
    const { readImage } = await import("@tauri-apps/plugin-clipboard-manager");
    const image = await readImage();
    const [{ width, height }, rgba] = await Promise.all([image.size(), image.rgba()]);
    if (!width || !height || rgba.length < width * height * 4) return null;

    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx) return null;
    ctx.putImageData(new ImageData(new Uint8ClampedArray(rgba), width, height), 0, 0);

    const blob = await new Promise<Blob | null>((resolve) =>
      canvas.toBlob(resolve, "image/png"),
    );
    if (!blob) return null;

    const name = "pasted-image.png";
    const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
    const stored = await unwrap(commands.saveAttachment(workstreamId, name, bytes));
    return { path: stored, name, kind: "image" };
  } catch {
    return null;
  }
}
