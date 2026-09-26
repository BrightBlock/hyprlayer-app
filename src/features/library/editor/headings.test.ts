import { describe, expect, it } from "vitest";

import { renderMarkdown } from "@/lib/markdown";
import { extractHeadings } from "./headings";

describe("extractHeadings", () => {
  // Failure mode 1: an H1 is included, or an H5 appears.
  it("excludes H1 and H5, keeps H2-H4", () => {
    const doc = ["# H1", "## H2", "### H3", "#### H4", "##### H5", "body"].join("\n");
    expect(extractHeadings(doc).map((h) => [h.level, h.text])).toEqual([
      [2, "H2"],
      [3, "H3"],
      [4, "H4"],
    ]);
  });

  // Failure mode 2: SetextHeading2 is missed.
  it("includes a setext (underline) H2, excludes a setext H1", () => {
    const doc = ["Title", "=====", "", "Sub", "-----", "", "body"].join("\n");
    expect(extractHeadings(doc)).toEqual([{ level: 2, text: "Sub", from: doc.indexOf("Sub"), line: 4 }]);
  });

  // Failure mode 3: a `## x` inside a fenced block is counted.
  it("does not count a heading-looking line inside a fenced code block", () => {
    const doc = ["## Real", "```", "## Fake", "```", "body"].join("\n");
    expect(extractHeadings(doc).map((h) => h.text)).toEqual(["Real"]);
  });

  // Failure mode 4: inline marks (**, `, link URL) leak into the text.
  it("strips inline emphasis, code and link marks from the heading text", () => {
    const doc = "## Bold **text** and `code` and [link](http://example.com)";
    expect(extractHeadings(doc)).toEqual([{ level: 2, text: "Bold text and code and link", from: 0, line: 1 }]);
  });

  // Failure mode 5: closing ## marks leak.
  it("strips a closing ATX mark sequence", () => {
    expect(extractHeadings("## Heading ##\nbody")).toEqual([{ level: 2, text: "Heading", from: 0, line: 1 }]);
  });

  // Failure mode 6: an empty heading produces an empty item.
  it("skips a heading with no text", () => {
    expect(extractHeadings("##\nbody")).toEqual([]);
  });

  // Failure mode 7: `from` is not the heading line's start.
  it("uses the heading's line start for `from`, not the mark's position", () => {
    const doc = "body\n\n  ## Indented\nmore";
    expect(extractHeadings(doc)).toEqual([{ level: 2, text: "Indented", from: doc.indexOf("  ##"), line: 3 }]);
  });

  // Failure mode 8: a heading's line disagrees with the preview's data-line, so the
  // scrollspy and contents jumps can't find it in the rendered doc.
  it("reports the same line the preview stamps on the heading", () => {
    const doc = ["intro", "", "## One", "", "```", "## not", "```", "", "Two", "---", "", "#### Three"].join("\n");
    const { html } = renderMarkdown(doc);
    for (const h of extractHeadings(doc)) {
      expect(html).toContain(`<h${h.level} data-line="${h.line}">`);
    }
    expect(extractHeadings(doc).map((h) => h.line)).toEqual([3, 9, 12]);
  });
});
