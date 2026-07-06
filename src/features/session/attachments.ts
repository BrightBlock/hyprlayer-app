/**
 * Prompt attachments — screenshots and files the user drops onto, or pastes
 * into, an agent composer. Volatile sources (a macOS screenshot dragged off the
 * capture thumbnail lives under `/var/folders/.../TemporaryItems/` and is reaped
 * moments later; clipboard image bytes have no path at all) are persisted into
 * the workstream's `.hyprlayer/attachments/` via the backend so they survive and
 * the agent — whose cwd is the worktree — can `Read` them.
 *
 * The path the agent receives is never shown in the transcript: the composer and
 * the conversation render a pin card instead (see AttachmentCard).
 */

import { convertFileSrc } from "@tauri-apps/api/core";

import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { baseName } from "./toolDisplay";

export type AttachmentKind = "image" | "file";

export type Attachment = {
  /** Stable absolute path the agent reads — for images this is the copy inside
   *  `<worktree>/.hyprlayer/attachments/`, never the volatile source. */
  path: string;
  /** Display filename (the original basename). */
  name: string;
  kind: AttachmentKind;
};

/** Image file extensions we treat as screenshots — the single source for both
 *  drop/paste detection and the "Add screenshots" file-dialog filter. */
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

/** A webview-loadable URL for a local image file (Tauri asset protocol). */
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

/** A sensible filename for a pasted image blob, which usually arrives nameless. */
export function clipboardImageName(file: File): string {
  if (file.name && file.name !== "image.png") return file.name;
  const ext = EXT_FOR_MIME[file.type] ?? "png";
  return `pasted-image.${ext}`;
}

/**
 * Persist dropped file paths into the workstream. Images (including volatile
 * macOS screencapture temp files) are COPIED into the worktree's attachments
 * dir so they outlive the OS reaping the source; other files are referenced in
 * place (they're usually real project files at a path worth keeping). Sources
 * that can't be read are skipped rather than failing the whole drop.
 */
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
        return null; // unreadable / vanished source — skip it
      }
    }),
  );
  return imported.filter((a): a is Attachment => a !== null);
}

/** Persist a pasted clipboard image into the workstream's attachments dir. */
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
