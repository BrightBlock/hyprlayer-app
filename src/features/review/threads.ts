import type { ReviewComment, ReviewNote, ReviewNoteKind } from "@/lib/bindings";

export type Thread = { root: ReviewComment; replies: ReviewComment[] };

export const NOTE_KIND_LABEL: Record<ReviewNoteKind, string> = {
  note: "Note",
  needs_your_call: "Needs your call",
  outside_plan_scope: "Outside plan scope",
};

const byCreated = (a: ReviewComment, b: ReviewComment) => (a.createdAt ?? 0) - (b.createdAt ?? 0);

// The server keeps the parent it was given, so a reply to a reply still belongs to the root's thread.
export function groupThreads(comments: ReviewComment[]): Thread[] {
  const byId = new Map(comments.map((c) => [c.id, c]));
  const rootOf = (c: ReviewComment) => {
    let cur = c;
    const seen = new Set<string>();
    while (cur.parentId && !seen.has(cur.id)) {
      seen.add(cur.id);
      const parent = byId.get(cur.parentId);
      if (!parent) break;
      cur = parent;
    }
    return cur;
  };
  const threads = new Map<string, Thread>();
  for (const c of [...comments].sort(byCreated)) {
    const root = rootOf(c);
    const thread = threads.get(root.id) ?? { root, replies: [] };
    if (c !== root) thread.replies.push(c);
    threads.set(root.id, thread);
  }
  return [...threads.values()];
}

export function threadSize(t: Thread): number {
  return 1 + t.replies.length;
}

// A thread is resolved through its root, and a note's replies through the note.
export function openCountsByPath(threads: Thread[], notes: ReviewNote[]): Map<string, number> {
  const counts = new Map<string, number>();
  const add = (path: string, n: number) => counts.set(path, (counts.get(path) ?? 0) + n);
  const resolvedNotes = new Set(notes.filter((n) => n.resolved).map((n) => n.id));
  for (const t of threads) {
    const noteId = t.root.replyToNoteId;
    if (!t.root.resolved && !(noteId && resolvedNotes.has(noteId))) add(t.root.path, threadSize(t));
  }
  for (const n of notes) if (!n.resolved) add(n.path, 1);
  return counts;
}
