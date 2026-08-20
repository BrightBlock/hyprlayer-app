import { basename } from "@/lib/path";

/** How every surface writes a rename: `old → new`, so a moved file reads as the
 *  one that moved rather than as a stranger arriving at an unfamiliar name. A
 *  non-rename (`oldPath` null) renders the path alone.
 *
 *  `short` trims both sides to their basenames, for the Changes tab's tree mode
 *  where the folder is already the row above.
 */
export function renamedLabel(path: string, oldPath: string | null, short = false): string {
  const shown = short ? basename(path) : path;
  if (!oldPath) return shown;
  return `${short ? basename(oldPath) : oldPath} → ${shown}`;
}
