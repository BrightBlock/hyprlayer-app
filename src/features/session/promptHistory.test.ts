import { describe, expect, it } from "vitest";

import {
  IDLE_NAV,
  MAX_HISTORY,
  recallAt,
  recallNext,
  recallPrev,
  stackWindow,
  userPrompts,
  type HistoryNav,
} from "./promptHistory";
import type { TranscriptItem } from "./transcript";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function userItem(text: string, id = text): Extract<TranscriptItem, { kind: "user" }> {
  return { kind: "user", id, text, stage: null, attachments: [], createdAt: null };
}

function assistantItem(text: string, id = text): Extract<TranscriptItem, { kind: "assistant" }> {
  return { kind: "assistant", id, text, streaming: false };
}

function nav(index: number | null, stash = ""): HistoryNav {
  return { index, stash };
}

// ---------------------------------------------------------------------------
// userPrompts
// ---------------------------------------------------------------------------

describe("userPrompts", () => {
  it("returns [] for an empty transcript", () => {
    expect(userPrompts([])).toEqual([]);
  });

  it("returns [] when there are no user items", () => {
    const items: TranscriptItem[] = [assistantItem("hello")];
    expect(userPrompts(items)).toEqual([]);
  });

  it("filters to kind=user and returns text oldest→newest", () => {
    const items: TranscriptItem[] = [
      userItem("first"),
      assistantItem("reply 1"),
      userItem("second"),
      assistantItem("reply 2"),
      userItem("third"),
    ];
    expect(userPrompts(items)).toEqual(["first", "second", "third"]);
  });

  it("collapses exact consecutive duplicates (bash ignoredups)", () => {
    const items: TranscriptItem[] = [
      userItem("hello", "u1"),
      userItem("hello", "u2"),
      userItem("world", "u3"),
      userItem("world", "u4"),
      userItem("world", "u5"),
    ];
    expect(userPrompts(items)).toEqual(["hello", "world"]);
  });

  it("does NOT collapse non-consecutive duplicates", () => {
    const items: TranscriptItem[] = [
      userItem("a", "u1"),
      userItem("b", "u2"),
      userItem("a", "u3"),
    ];
    expect(userPrompts(items)).toEqual(["a", "b", "a"]);
  });

  it("returns the text verbatim, including a stage slash-command prefix", () => {
    const items: TranscriptItem[] = [
      userItem("/create_plan Build a rocket", "u1"),
      userItem("follow-up", "u2"),
    ];
    expect(userPrompts(items)).toEqual(["/create_plan Build a rocket", "follow-up"]);
  });

  it("caps history to the most recent MAX_HISTORY, aging out the oldest", () => {
    const total = MAX_HISTORY + 7;
    const items: TranscriptItem[] = Array.from({ length: total }, (_, i) =>
      userItem(`prompt ${i}`, `u${i}`),
    );
    const result = userPrompts(items);
    expect(result).toHaveLength(MAX_HISTORY);
    // The window holds the newest entries; the oldest 7 fell off the back.
    expect(result[0]).toBe(`prompt ${total - MAX_HISTORY}`);
    expect(result[result.length - 1]).toBe(`prompt ${total - 1}`);
  });

  it("returns all prompts unchanged when at or below the cap", () => {
    const items: TranscriptItem[] = Array.from({ length: MAX_HISTORY }, (_, i) =>
      userItem(`prompt ${i}`, `u${i}`),
    );
    expect(userPrompts(items)).toHaveLength(MAX_HISTORY);
  });

  it("applies the cap AFTER collapsing consecutive duplicates", () => {
    // 2*MAX_HISTORY raw items, but every pair is a duplicate → MAX_HISTORY unique,
    // exactly at the cap (nothing aged out).
    const items: TranscriptItem[] = [];
    for (let i = 0; i < MAX_HISTORY; i += 1) {
      items.push(userItem(`p${i}`, `a${i}`), userItem(`p${i}`, `b${i}`));
    }
    const result = userPrompts(items);
    expect(result).toHaveLength(MAX_HISTORY);
    expect(result[0]).toBe("p0");
  });
});

// ---------------------------------------------------------------------------
// recallPrev
// ---------------------------------------------------------------------------

describe("recallPrev", () => {
  it("returns null for empty history", () => {
    expect(recallPrev([], IDLE_NAV, "anything")).toBeNull();
  });

  it("from idle: stashes the live draft and lands on the newest prompt", () => {
    const prompts = ["first", "second", "third"];
    const result = recallPrev(prompts, IDLE_NAV, "in-progress");
    expect(result).toEqual({
      nav: { index: 2, stash: "in-progress" },
      draft: "third",
    });
  });

  it("steps to the next older prompt on repeated calls", () => {
    const prompts = ["first", "second", "third"];
    const r1 = recallPrev(prompts, IDLE_NAV, "")!;
    expect(r1.nav.index).toBe(2);
    const r2 = recallPrev(prompts, r1.nav, r1.draft)!;
    expect(r2.nav.index).toBe(1);
    expect(r2.draft).toBe("second");
    const r3 = recallPrev(prompts, r2.nav, r2.draft)!;
    expect(r3.nav.index).toBe(0);
    expect(r3.draft).toBe("first");
  });

  it("returns null at index 0 (already at the oldest)", () => {
    const prompts = ["only"];
    const atOldest = nav(0, "stash");
    expect(recallPrev(prompts, atOldest, "only")).toBeNull();
  });

  it("preserves the stash when stepping further back", () => {
    const prompts = ["a", "b", "c"];
    const start = nav(2, "my-stash");
    const result = recallPrev(prompts, start, "b")!;
    expect(result.nav.stash).toBe("my-stash");
  });
});

// ---------------------------------------------------------------------------
// recallNext
// ---------------------------------------------------------------------------

describe("recallNext", () => {
  it("returns null when not navigating (index === null)", () => {
    expect(recallNext(["a", "b"], IDLE_NAV, "")).toBeNull();
  });

  it("steps to the next newer prompt", () => {
    const prompts = ["a", "b", "c"];
    const result = recallNext(prompts, nav(0, "stash"), "a");
    expect(result).toEqual({ nav: { index: 1, stash: "stash" }, draft: "b" });
  });

  it("exits navigation and restores the stash when stepping past the newest", () => {
    const prompts = ["a", "b", "c"];
    const result = recallNext(prompts, nav(2, "my-draft"), "c");
    expect(result).toEqual({ nav: IDLE_NAV, draft: "my-draft" });
  });

  it("restores an empty stash correctly", () => {
    const prompts = ["hello"];
    const result = recallNext(prompts, nav(0, ""), "hello");
    expect(result).toEqual({ nav: IDLE_NAV, draft: "" });
  });
});

// ---------------------------------------------------------------------------
// recallAt
// ---------------------------------------------------------------------------

describe("recallAt", () => {
  const prompts = ["alpha", "beta", "gamma"];

  it("returns null for an out-of-range index (negative)", () => {
    expect(recallAt(prompts, IDLE_NAV, "d", -1)).toBeNull();
  });

  it("returns null for an out-of-range index (>= length)", () => {
    expect(recallAt(prompts, IDLE_NAV, "d", 3)).toBeNull();
  });

  it("on first entry (IDLE_NAV): stashes the live draft and lands on the given index", () => {
    const result = recallAt(prompts, IDLE_NAV, "in-progress", 1);
    expect(result).toEqual({ nav: { index: 1, stash: "in-progress" }, draft: "beta" });
  });

  it("when already navigating: keeps the existing stash", () => {
    const alreadyNav = nav(2, "original-stash");
    const result = recallAt(prompts, alreadyNav, "gamma", 0);
    expect(result).toEqual({ nav: { index: 0, stash: "original-stash" }, draft: "alpha" });
  });

  it("can recall the first element (index 0)", () => {
    const result = recallAt(prompts, IDLE_NAV, "typing…", 0);
    expect(result).toEqual({ nav: { index: 0, stash: "typing…" }, draft: "alpha" });
  });

  it("can recall the last element", () => {
    const result = recallAt(prompts, IDLE_NAV, "", 2);
    expect(result).toEqual({ nav: { index: 2, stash: "" }, draft: "gamma" });
  });
});

// ---------------------------------------------------------------------------
// stackWindow
// ---------------------------------------------------------------------------

describe("stackWindow", () => {
  it("returns [0, count) when count <= max", () => {
    expect(stackWindow(3, null, 8)).toEqual({ start: 0, end: 3 });
    expect(stackWindow(8, null, 8)).toEqual({ start: 0, end: 8 });
  });

  it("returns the most-recent window when count > max and active is inside", () => {
    // count=12, max=8 → start=4, end=12; active=7 is inside
    expect(stackWindow(12, 7, 8)).toEqual({ start: 4, end: 12 });
  });

  it("returns the most-recent window when active is null", () => {
    expect(stackWindow(10, null, 8)).toEqual({ start: 2, end: 10 });
  });

  it("shifts up to reveal an out-of-window active line", () => {
    // count=12, max=8 → default start=4; active=1 is outside → start=1
    expect(stackWindow(12, 1, 8)).toEqual({ start: 1, end: 9 });
  });

  it("clamps the start at 0 when active=0 forces it there", () => {
    expect(stackWindow(10, 0, 8)).toEqual({ start: 0, end: 8 });
  });

  it("does not shift when active equals the last-in-window index", () => {
    // count=10, max=8 → start=2, end=10; active=9 (newest) is inside
    expect(stackWindow(10, 9, 8)).toEqual({ start: 2, end: 10 });
  });

  it("handles count === 0", () => {
    expect(stackWindow(0, null, 8)).toEqual({ start: 0, end: 0 });
  });
});
