import { docKindForPath, sessionDocs } from "./toolDisplay";
import type { TranscriptItem } from "./transcript";
import type { WrittenDocs } from "./writtenDocs";

// Live, the first user turn holds the attachments; restored from the log, core's
// `Relevant documents:` list (pipeline::append_docs) in its text.
function referencedPlan(items: TranscriptItem[]): string | null {
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

function writtenPlan(items: TranscriptItem[], written: WrittenDocs): string | null {
  return sessionDocs(items, written).filter((d) => d.kind === "plan").pop()?.path ?? null;
}

export function planDocForSession(
  items: TranscriptItem[],
  stage: string,
  written: WrittenDocs,
): string | null {
  if (stage === "implement") return referencedPlan(items) ?? writtenPlan(items, written);
  return writtenPlan(items, written) ?? referencedPlan(items);
}
