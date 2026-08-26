import { basename } from "@/lib/path";

export function renamedLabel(path: string, oldPath: string | null, short = false): string {
  const shown = short ? basename(path) : path;
  if (!oldPath) return shown;
  return `${short ? basename(oldPath) : oldPath} → ${shown}`;
}
