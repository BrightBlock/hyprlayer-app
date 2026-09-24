import { docKindForPath } from "./toolDisplay";
import type { TranscriptItem } from "./transcript";

// Live, the first user turn holds the attachments; restored from the log, core's
// `Relevant documents:` list (pipeline::append_docs) in its text.
export function referencedPlan(items: TranscriptItem[]): string | null {
  const first = items.find((i) => i.kind === "user");
  if (!first || first.kind !== "user") return null;
  const attached = first.attachments.find((a) => docKindForPath(a.path) === "plan");
  if (attached) return attached.path;
  const [, docs = ""] = first.text.split(/\n\nRelevant documents:\n/);
  for (const line of docs.split("\n")) {
    const path = line.replace(/^- /, "").trim();
    if (docKindForPath(path) === "plan") return path;
  }
  const arg = /^\/implement_plan\s+(\S+\.md)/m.exec(first.text)?.[1];
  return arg && docKindForPath(arg) === "plan" ? arg : null;
}
