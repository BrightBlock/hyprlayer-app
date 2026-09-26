import { renderMarkdown } from "@/lib/markdown";

export type DocHeading = { level: 2 | 3 | 4; text: string; from: number; line: number };

// Reads the preview's cached parse, so opening a doc parses it once.
export function extractHeadings(body: string): DocHeading[] {
  const out: DocHeading[] = [];
  let line = 1;
  let from = 0;
  for (const h of renderMarkdown(body).headings) {
    if (h.level < 2 || h.level > 4 || !h.text) continue;
    for (; line < h.line; line++) from = body.indexOf("\n", from) + 1;
    out.push({ level: h.level as 2 | 3 | 4, text: h.text, from, line: h.line });
  }
  return out;
}
