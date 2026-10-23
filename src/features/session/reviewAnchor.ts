import { toolFilePath } from "./toolDisplay";
import type { TranscriptItem } from "./transcript";

// Every harness reports its file edits under these names.
const EDIT_TOOLS = new Set(["Edit", "Write", "MultiEdit", "NotebookEdit"]);

// Items are immutable, so a streaming transcript parses each tool's input once.
const editsByItem = new WeakMap<TranscriptItem, boolean>();

function editsWorktree(item: TranscriptItem, root: string): boolean {
  if (item.kind !== "tool" || !EDIT_TOOLS.has(item.name)) return false;
  let edits = editsByItem.get(item);
  if (edits === undefined) {
    const path = toolFilePath(item.name, item.input)?.replace(/\\/g, "/") ?? null;
    const relative = path !== null && !/^([A-Za-z]:)?\//.test(path);
    const rel = path === null ? null : relative ? path : path.startsWith(`${root}/`) ? path.slice(root.length + 1) : null;
    edits = rel !== null && rel !== "thoughts" && !rel.startsWith("thoughts/");
    editsByItem.set(item, edits);
  }
  return edits;
}

/**
 * The user message the review card sits before: the one opening the turn after the last finished
 * turn that edited the worktree. Null when that turn is the newest, so the card closes the transcript.
 */
export function reviewAnchor(items: TranscriptItem[], busy: boolean, worktree: string): string | null {
  const root = worktree.replace(/\\/g, "/").replace(/\/+$/, "");
  const starts: string[] = [];
  const edited: boolean[] = [];
  for (const item of items) {
    if (item.kind === "user") {
      starts.push(item.id);
      edited.push(false);
    } else if (edited.length > 0 && !edited[edited.length - 1] && editsWorktree(item, root)) {
      edited[edited.length - 1] = true;
    }
  }
  const finished = busy ? starts.length - 1 : starts.length;
  let last = -1;
  for (let i = finished - 1; i >= 0; i--) {
    if (edited[i]) {
      last = i;
      break;
    }
  }
  // Nothing seen edited: a running turn still opens below the card.
  if (last === -1) return busy && finished >= 1 ? starts[finished] : null;
  return last + 1 < starts.length ? starts[last + 1] : null;
}
