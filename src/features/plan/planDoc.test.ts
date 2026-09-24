import { describe, expect, it } from "vitest";

import { parsePlanDoc, toggleTask } from "./planDoc";

const PATH = "/t/repos/app/shared/plans/2026-09-24-og-images.md";

const PLAN = `---
title: 2026-09-24-og-images
type: plan
status: draft
author: jt
---

# OG images route

## Overview

Serve generated OG images from a route.

- [ ] not a task: before the first phase

## Phase 1: Scaffold

### Changes Required:

#### 1. Route

\`\`\`md
- [ ] fenced, not a task
\`\`\`

### Success Criteria:

#### Automated Verification:
- [x] Types check: \`pnpm typecheck\`
- [ ] Lint passes <!-- verified live -->

---

## Phase 2: Build

- [ ] Port OG Images.html to the route
  continued on a second line
* [X] Wire the image slot
+ [~] Partially there
    - [ ] Nested item

## Testing Strategy

- [ ] not a task: after the last phase
`;

describe("parsePlanDoc", () => {
  it("reads title, status, author and overview", () => {
    const doc = parsePlanDoc(PLAN, PATH);
    expect(doc.title).toBe("OG images route");
    expect(doc.status).toBe("draft");
    expect(doc.author).toBe("jt");
    expect(doc.overview).toBe("Serve generated OG images from a route.\n\n- [ ] not a task: before the first phase");
  });

  it("falls back to frontmatter title, then filename", () => {
    expect(parsePlanDoc("---\ntitle: From FM\n---\n\n## Phase 1: A\n", PATH).title).toBe("From FM");
    expect(parsePlanDoc("## Phase 1: A\n", PATH).title).toBe("2026-09-24-og-images");
  });

  it("yields no phases for a doc without phase headings", () => {
    expect(parsePlanDoc("# T\n\n- [ ] loose\n", PATH).phases).toEqual([]);
  });

  it("parses phases and ignores checkboxes outside them and inside fences", () => {
    const doc = parsePlanDoc(PLAN, PATH);
    expect(doc.phases.map((p) => [p.label, p.name])).toEqual([
      ["1", "Scaffold"],
      ["2", "Build"],
    ]);
    expect(doc.phases[0].tasks.map((t) => [t.key, t.title, t.checked])).toEqual([
      ["P1.1", "Types check: pnpm typecheck", true],
      ["P1.2", "Lint passes", false],
    ]);
  });

  it("handles bullet variants, [~], nested items and continuation lines", () => {
    const tasks = parsePlanDoc(PLAN, PATH).phases[1].tasks;
    expect(tasks.map((t) => [t.key, t.title, t.checked])).toEqual([
      ["P2.1", "Port OG Images.html to the route", false],
      ["P2.2", "Wire the image slot", true],
      ["P2.3", "Partially there", false],
      ["P2.4", "Nested item", false],
    ]);
  });

  it("accepts em-dash, lettered and prefixed phase headings", () => {
    const doc = parsePlanDoc(
      "## Phase 2 — Frontend: tracker\n- [ ] a\n## Phase 4b: split\n- [ ] b\n# BRI-5 Phase 1: core\n- [ ] c\n",
      PATH,
    );
    expect(doc.phases.map((p) => [p.label, p.name])).toEqual([
      ["2", "Frontend: tracker"],
      ["4b", "split"],
      ["1", "core"],
    ]);
    expect(doc.phases[1].tasks[0].key).toBe("P4b.1");
  });

  it("parses CRLF input the same as LF", () => {
    const crlf = parsePlanDoc(PLAN.replace(/\n/g, "\r\n"), PATH);
    const lf = parsePlanDoc(PLAN, PATH);
    expect(crlf.phases).toEqual(lf.phases);
    expect(crlf.title).toBe(lf.title);
  });
});

describe("toggleTask", () => {
  const doc = parsePlanDoc(PLAN, PATH);
  const port = doc.phases[1].tasks[0];

  it("flips exactly one line and leaves every other byte alone", () => {
    const next = toggleTask(PLAN, port, true);
    expect(next).not.toBeNull();
    const before = PLAN.split("\n");
    const after = next!.split("\n");
    expect(after.length).toBe(before.length);
    const changed = after.flatMap((l, i) => (l === before[i] ? [] : [i]));
    expect(changed).toHaveLength(1);
    expect(after[changed[0]]).toBe("- [x] Port OG Images.html to the route");
  });

  it("is idempotent", () => {
    const once = toggleTask(PLAN, port, true)!;
    expect(toggleTask(once, port, true)).toBe(once);
  });

  it("unchecks", () => {
    const wire = doc.phases[1].tasks[1];
    expect(toggleTask(PLAN, wire, false)).toContain("* [ ] Wire the image slot");
  });

  it("preserves CRLF", () => {
    const crlf = PLAN.replace(/\n/g, "\r\n");
    const next = toggleTask(crlf, port, true)!;
    expect(next).toBe(crlf.replace("- [ ] Port OG Images.html", "- [x] Port OG Images.html"));
  });

  it("follows the title when an edit shifted the key", () => {
    const edited = PLAN.replace("## Phase 2: Build\n\n", "## Phase 2: Build\n\n- [ ] Inserted first\n");
    const next = toggleTask(edited, port, true)!;
    expect(next).toContain("- [ ] Inserted first");
    expect(next).toContain("- [x] Port OG Images.html to the route");
  });

  it("refuses when no task carries the title", () => {
    expect(toggleTask(PLAN, { key: "P2.1", title: "renamed away" }, true)).toBeNull();
  });
});
