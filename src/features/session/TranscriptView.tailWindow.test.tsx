// @vitest-environment jsdom
import { createRef } from "react";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TranscriptItem } from "./transcript";
import { TranscriptView, tailStart, topLevelIndexOf, type TranscriptHandle } from "./TranscriptView";

type Observed = { callback: IntersectionObserverCallback; target: Element | null };
let observers: Observed[] = [];

class FakeIntersectionObserver {
  private entry: Observed;
  constructor(callback: IntersectionObserverCallback) {
    this.entry = { callback, target: null };
    observers.push(this.entry);
  }
  observe(target: Element) {
    this.entry.target = target;
  }
  disconnect() {
    observers = observers.filter((o) => o !== this.entry);
  }
  unobserve() {}
  takeRecords() {
    return [];
  }
}

function reachTop() {
  const live = observers.filter((o) => o.target?.isConnected);
  act(() => {
    for (const o of live) {
      o.callback([{ isIntersecting: true } as IntersectionObserverEntry], o as unknown as IntersectionObserver);
    }
  });
}

const errors = (n: number): TranscriptItem[] =>
  Array.from({ length: n }, (_, i) => ({ kind: "error" as const, id: `e${i}`, detail: `row ${i}`, raw: "" }));

function view(items: TranscriptItem[], ref = createRef<TranscriptHandle>()) {
  return (
    <TranscriptView
      ref={ref}
      items={items}
      busy={false}
      permission={{ onAllow: () => {}, onAllowForSession: () => {}, onDeny: () => {} }}
      prompt={{ onRewind: () => {} }}
    />
  );
}

beforeEach(() => {
  observers = [];
  vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
  vi.stubGlobal("CSS", { escape: (s: string) => s });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("transcript tail window", () => {
  it("mounts only the newest rows of a long session", () => {
    const { queryByText } = render(view(errors(300)));
    expect(queryByText("row 299")).not.toBeNull();
    expect(queryByText("row 180")).not.toBeNull();
    expect(queryByText("row 179")).toBeNull();
    expect(queryByText("row 0")).toBeNull();
  });

  it("shows earlier rows as the reader nears the top", () => {
    const { queryByText } = render(view(errors(300)));
    reachTop();
    expect(queryByText("row 60")).not.toBeNull();
    expect(queryByText("row 59")).toBeNull();
    reachTop();
    expect(queryByText("row 0")).not.toBeNull();
  });

  it("keeps the first shown row while new rows stream in below", () => {
    const items = errors(300);
    const { queryByText, rerender } = render(view(items));
    rerender(view([...items, ...errors(340).slice(300)]));
    expect(queryByText("row 180")).not.toBeNull();
    expect(queryByText("row 339")).not.toBeNull();
  });

  it("mounts an older row a jump targets", () => {
    const scrolled = vi.fn();
    Element.prototype.scrollIntoView = scrolled;
    const ref = createRef<TranscriptHandle>();
    const { queryByText } = render(view(errors(300), ref));
    expect(queryByText("row 10")).toBeNull();
    act(() => ref.current!.scrollToItem("e10"));
    expect(queryByText("row 10")).not.toBeNull();
    expect(scrolled).toHaveBeenCalled();
  });

  it("renders a short session whole", () => {
    const { queryByText } = render(view(errors(40)));
    expect(queryByText("row 0")).not.toBeNull();
  });
});

describe("tailStart", () => {
  it("starts at the row already shown, and falls back to the tail once it is gone", () => {
    const items = errors(300);
    expect(tailStart(items, null)).toBe(180);
    expect(tailStart(items, "e42")).toBe(42);
    expect(tailStart(items, "gone")).toBe(180);
  });
});

describe("topLevelIndexOf", () => {
  it("climbs from a subagent's item to the row that holds it", () => {
    const parent: TranscriptItem = {
      kind: "tool",
      id: "t",
      toolUseId: "tu1",
      name: "Task",
      input: {},
      result: null,
    } as unknown as TranscriptItem;
    const child: TranscriptItem = {
      kind: "assistant",
      id: "c",
      text: "inner",
      parentId: "tu1",
    } as unknown as TranscriptItem;
    const items = [...errors(3), parent, child];
    const topLevel = [...errors(3), parent];
    expect(topLevelIndexOf(items, topLevel, (it) => it.id === "c")).toBe(3);
    expect(topLevelIndexOf(items, topLevel, (it) => it.id === "nope")).toBe(-1);
  });
});
