import { describe, expect, it } from "vitest";

import { parseReviewMessage } from "./reviewMessage";

const FOOTER = "Address each comment, then stop. Files you change go back to unviewed in the review.";

// Byte for byte the message `review_lifecycle` pins in its golden.
const GOLDEN = [
  "Review · Changes requested · 2 / 7 viewed · 6 comments",
  "",
  "Tighten the constants before this ships.",
  "",
  "- `src/edit.rs:4` Why 3, and not 30?",
  "- `src/edit.rs:4` It balances the sum.",
  "- `src/gone.rs:2 (removed line)` Why is this here?",
  "- `src/gone.rs:2` Why is this here?",
  "- `src/new_name.rs:4 (removed line)` Why is this here?",
  "- `src/edit.rs:7` Name this constant.",
  "  ",
  "  It also needs a doc comment.",
  "",
  FOOTER,
].join("\n");

const message = (...lines: string[]) => lines.join("\n");

describe("parseReviewMessage", () => {
  it("reads the golden message from the E2E scenario", () => {
    expect(parseReviewMessage(GOLDEN)).toEqual({
      viewed: 2,
      files: 7,
      count: 6,
      summary: "Tighten the constants before this ships.",
      items: [
        { path: "src/edit.rs", line: 4, removed: false, body: "Why 3, and not 30?" },
        { path: "src/edit.rs", line: 4, removed: false, body: "It balances the sum." },
        { path: "src/gone.rs", line: 2, removed: true, body: "Why is this here?" },
        { path: "src/gone.rs", line: 2, removed: false, body: "Why is this here?" },
        { path: "src/new_name.rs", line: 4, removed: true, body: "Why is this here?" },
        { path: "src/edit.rs", line: 7, removed: false, body: "Name this constant.\n\nIt also needs a doc comment." },
      ],
    });
  });

  it("reads a message with no summary", () => {
    const parsed = parseReviewMessage(
      message("Review · Changes requested · 0 / 3 viewed · 1 comment", "", "- `a.rs:1` Fix it.", "", FOOTER),
    );
    expect(parsed).toEqual({
      viewed: 0,
      files: 3,
      count: 1,
      summary: null,
      items: [{ path: "a.rs", line: 1, removed: false, body: "Fix it." }],
    });
  });

  it("reads a summary alone, with no comments", () => {
    const parsed = parseReviewMessage(
      message("Review · Changes requested · 3 / 3 viewed · 0 comments", "", "Split the module.", "", FOOTER),
    );
    expect(parsed?.summary).toBe("Split the module.");
    expect(parsed?.items).toEqual([]);
    const bare = parseReviewMessage(message("Review · Changes requested · 0 / 1 viewed · 0 comments", "", FOOTER));
    expect(bare).toEqual({ viewed: 0, files: 1, count: 0, summary: null, items: [] });
  });

  it("keeps a multi-line summary whole, even when a line looks like a comment", () => {
    const parsed = parseReviewMessage(
      message(
        "Review · Changes requested · 1 / 2 viewed · 1 comment",
        "",
        "Two things:",
        "- `x.rs:9` was the old approach",
        "",
        "and the rest.",
        "",
        "- `b.rs:3` Rename.",
        "",
        FOOTER,
      ),
    );
    expect(parsed?.summary).toBe("Two things:\n- `x.rs:9` was the old approach\n\nand the rest.");
    expect(parsed?.items).toEqual([{ path: "b.rs", line: 3, removed: false, body: "Rename." }]);
  });

  it("joins multi-line bodies and keeps their inner indentation", () => {
    const parsed = parseReviewMessage(
      message(
        "Review · Changes requested · 0 / 1 viewed · 2 comments",
        "",
        "- `a.rs:1` First line.",
        "  Second line.",
        "    indented code",
        "- `a.rs:2` Next.",
        "",
        FOOTER,
      ),
    );
    expect(parsed?.items.map((i) => i.body)).toEqual(["First line.\nSecond line.\n  indented code", "Next."]);
  });

  it("reads a note reply: the quoted note above the answer", () => {
    const parsed = parseReviewMessage(
      message(
        "Review · Changes requested · 0 / 1 viewed · 2 comments",
        "",
        "- `a.rs:6` Reply to note [outside_plan_scope]:",
        "  > Is 20 right?",
        "  >",
        "  > Confirm the constant.",
        "  ",
        "  > Yes, keep 20.",
        "  Add a doc comment.",
        "- `a.rs:9` Plain.",
        "",
        FOOTER,
      ),
    );
    expect(parsed?.items).toEqual([
      {
        path: "a.rs",
        line: 6,
        removed: false,
        body: "> Yes, keep 20.\nAdd a doc comment.",
        note: { kind: "outside_plan_scope", body: "Is 20 right?\n\nConfirm the constant." },
      },
      { path: "a.rs", line: 9, removed: false, body: "Plain." },
    ]);
  });

  it("keeps a body that only looks like a note reply", () => {
    const parsed = parseReviewMessage(
      message(
        "Review · Changes requested · 0 / 1 viewed · 4 comments",
        "",
        "- `a.rs:1` Reply to note [shout]:",
        "  > q",
        "  ",
        "  a",
        "- `a.rs:4` Reply to note [constructor]:",
        "  > q",
        "  ",
        "  a",
        "- `a.rs:2` Reply to note [note]:",
        "  not quoted",
        "- `a.rs:3` Reply to note [note]:",
        "  > q",
        "",
        FOOTER,
      ),
    );
    expect(parsed?.items.map((i) => i.note)).toEqual([undefined, undefined, undefined, undefined]);
    expect(parsed?.items[0].body).toBe("Reply to note [shout]:\n> q\n\na");
  });

  it("marks a removed line", () => {
    const parsed = parseReviewMessage(
      message("Review · Changes requested · 0 / 1 viewed · 1 comment", "", "- `old.rs:12 (removed line)` Why drop this?", "", FOOTER),
    );
    expect(parsed?.items).toEqual([{ path: "old.rs", line: 12, removed: true, body: "Why drop this?" }]);
  });

  it("reads paths with spaces and colons up to the last line number", () => {
    const parsed = parseReviewMessage(
      message(
        "Review · Changes requested · 0 / 2 viewed · 2 comments",
        "",
        "- `docs/my notes.md:3` Spaces.",
        "- `src/a:b:7.rs:41 (removed line)` Colons.",
        "",
        FOOTER,
      ),
    );
    expect(parsed?.items).toEqual([
      { path: "docs/my notes.md", line: 3, removed: false, body: "Spaces." },
      { path: "src/a:b:7.rs", line: 41, removed: true, body: "Colons." },
    ]);
  });

  it.each([
    ["plain text", "Please fix the tests."],
    ["an empty message", ""],
    ["a header alone", "Review · Changes requested · 0 / 1 viewed · 1 comment"],
    ["no footer", message("Review · Changes requested · 0 / 1 viewed · 1 comment", "", "- `a.rs:1` Fix.")],
    [
      "fewer comments than the header counts",
      message("Review · Changes requested · 0 / 1 viewed · 2 comments", "", "- `a.rs:1` Fix.", "", FOOTER),
    ],
    [
      "a location with no line number",
      message("Review · Changes requested · 0 / 1 viewed · 1 comment", "", "- `a.rs` Fix.", "", FOOTER),
    ],
    [
      "a comment with no body",
      message("Review · Changes requested · 0 / 1 viewed · 1 comment", "", "- `a.rs:1`", "", FOOTER),
    ],
    [
      "a different header",
      message("Review · Approved · 0 / 1 viewed · 1 comment", "", "- `a.rs:1` Fix.", "", FOOTER),
    ],
    ["the wrong plural", message("Review · Changes requested · 0 / 1 viewed · 1 comments", "", "- `a.rs:1` Fix.", "", FOOTER)],
  ])("falls back on %s", (_label, text) => {
    expect(parseReviewMessage(text)).toBeNull();
  });
});
