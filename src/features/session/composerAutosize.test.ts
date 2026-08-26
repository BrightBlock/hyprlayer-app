import { describe, expect, it } from "vitest";

import {
  fitComposerHeight,
  MAX_COMPOSER_HEIGHT,
  type SizableTextarea,
} from "./composerAutosize";

function fakeTextarea(
  rowsWhenUnobstructed: number,
  rowHeight = 24,
): SizableTextarea {
  const style = { height: "", overflowY: "" };
  return {
    style,
    get scrollHeight() {
      const rows =
        rowsWhenUnobstructed + (style.overflowY === "hidden" ? 0 : 1);
      return rows * rowHeight;
    },
  };
}

describe("fitComposerHeight", () => {
  it("grows the box to fit its content", () => {
    const ta = fakeTextarea(3);
    expect(fitComposerHeight(ta)).toBe(72);
    expect(ta.style.height).toBe("72px");
  });

  it("measures with the scrollbar off, so a wrap can't cost a phantom row", () => {
    const ta = fakeTextarea(2);
    expect(fitComposerHeight(ta)).toBe(48);
  });

  it("keeps the scrollbar off below the cap, so sizing settles in one pass", () => {
    const ta = fakeTextarea(3);
    fitComposerHeight(ta);
    expect(ta.style.overflowY).toBe("hidden");
    expect(fitComposerHeight(ta)).toBe(72);
    expect(ta.style.overflowY).toBe("hidden");
  });

  it("caps the height and hands scrolling back over at the cap", () => {
    const ta = fakeTextarea(40);
    expect(fitComposerHeight(ta)).toBe(MAX_COMPOSER_HEIGHT);
    expect(ta.style.height).toBe(`${MAX_COMPOSER_HEIGHT}px`);
    expect(ta.style.overflowY).toBe("auto");
  });

  it("shrinks again when the draft does", () => {
    const style = { height: "", overflowY: "" };
    let rows = 5;
    const ta: SizableTextarea = {
      style,
      get scrollHeight() {
        return rows * 24;
      },
    };
    fitComposerHeight(ta);
    expect(style.height).toBe("120px");
    rows = 1;
    fitComposerHeight(ta);
    expect(style.height).toBe("24px");
  });
});
