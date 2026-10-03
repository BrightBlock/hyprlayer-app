// Mirrors `crates/hyprlayer-core/src/review/message.rs`, which composes the message.

export type ReviewMessageItem = { path: string; line: number; removed: boolean; body: string };

export type ReviewMessage = {
  viewed: number;
  files: number;
  count: number;
  summary: string | null;
  items: ReviewMessageItem[];
};

const HEADER = /^Review · Changes requested · (\d+) \/ (\d+) viewed · (\d+) (comments?)$/;
// The path runs to the last `:<line>`, so it may hold spaces and colons, never a backtick.
const ITEM = /^- `([^`]+):(\d+)( \(removed line\))?` (.+)$/;
const FOOTER = "Address each comment, then stop. Files you change go back to unviewed in the review.";
const INDENT = "  ";

/** Null for anything the grammar didn't produce, so the caller shows the plain prompt. */
export function parseReviewMessage(text: string): ReviewMessage | null {
  const lines = text.split("\n");
  const head = HEADER.exec(lines[0]);
  if (!head || lines[1] !== "" || lines[lines.length - 1] !== FOOTER) return null;
  const count = Number(head[3]);
  if ((count === 1) !== (head[4] === "comment")) return null;

  // Read the items upward from the footer; the header's count says where the summary ends.
  const items: ReviewMessageItem[] = [];
  let start = lines.length - 1;
  if (count > 0) {
    if (lines[lines.length - 2] !== "") return null;
    let continuation: string[] = [];
    let i = lines.length - 3;
    for (; i >= 2 && items.length < count; i--) {
      const line = lines[i];
      const item = ITEM.exec(line);
      if (item) {
        items.unshift({
          path: item[1],
          line: Number(item[2]),
          removed: item[3] !== undefined,
          body: [item[4], ...continuation].join("\n"),
        });
        continuation = [];
      } else if (line.startsWith(INDENT)) {
        continuation.unshift(line.slice(INDENT.length));
      } else {
        return null;
      }
    }
    if (items.length < count || continuation.length > 0) return null;
    start = i + 1;
  }

  let summary: string | null = null;
  if (start > 2) {
    if (lines[start - 1] !== "" || start - 1 <= 2) return null;
    summary = lines.slice(2, start - 1).join("\n");
  }
  return { viewed: Number(head[1]), files: Number(head[2]), count, summary, items };
}
