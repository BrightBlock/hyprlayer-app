// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { installDragRegion, isDragRegion } from "./dragRegion";

vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({}) }));

const host = { startDragging: vi.fn(), toggleMaximize: vi.fn() };
let teardown: () => void;

function down(el: Element, x: number, y: number, detail = 1, button = 0) {
  const e = new MouseEvent("mousedown", {
    bubbles: true,
    cancelable: true,
    clientX: x,
    clientY: y,
    detail,
    button,
  });
  el.dispatchEvent(e);
  return e;
}
const move = (x: number, y: number) =>
  document.dispatchEvent(
    new MouseEvent("mousemove", { bubbles: true, clientX: x, clientY: y }),
  );
const up = () =>
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));

beforeEach(() => {
  host.startDragging.mockReset();
  host.toggleMaximize.mockReset();
  document.body.innerHTML = `
    <header id="bar" data-drag-region="">
      <span id="mark" data-drag-region="deep"><svg id="glyph"></svg></span>
      <button id="btn">x</button>
      <div id="plain"></div>
    </header>
    <div id="outside"></div>`;
  teardown = installDragRegion(document, host);
});
afterEach(() => teardown());

const $ = (id: string) => document.getElementById(id) as HTMLElement;

describe("isDragRegion", () => {
  it("matches a direct hit on a bare region but not its children", () => {
    expect(isDragRegion([$("bar")])).toBe(true);
    expect(isDragRegion([$("plain"), $("bar")])).toBe(false);
  });

  it("matches anywhere inside a deep region", () => {
    expect(isDragRegion([$("glyph"), $("mark"), $("bar")])).toBe(true);
  });

  it("lets a clickable element veto the region above it", () => {
    expect(isDragRegion([$("btn"), $("bar")])).toBe(false);
  });
});

describe("drag threshold", () => {
  it("does not grab the window on a plain press", () => {
    down($("bar"), 100, 5);
    expect(host.startDragging).not.toHaveBeenCalled();
  });

  it("leaves a press-and-release under the threshold a click", () => {
    down($("bar"), 100, 5);
    move(102, 6);
    up();
    move(400, 300);
    expect(host.startDragging).not.toHaveBeenCalled();
  });

  it("starts the drag once the pointer travels far enough", () => {
    down($("bar"), 100, 5);
    move(103, 5);
    expect(host.startDragging).not.toHaveBeenCalled();
    move(110, 5);
    expect(host.startDragging).toHaveBeenCalledTimes(1);
  });

  it("asks for the move only once per press", () => {
    down($("bar"), 100, 5);
    move(120, 5);
    move(140, 5);
    expect(host.startDragging).toHaveBeenCalledTimes(1);
  });

  it("ignores motion after a press outside any drag region", () => {
    down($("outside"), 100, 5);
    move(200, 100);
    expect(host.startDragging).not.toHaveBeenCalled();
  });

  it("ignores a press on a button inside the bar", () => {
    down($("btn"), 100, 5);
    move(200, 100);
    expect(host.startDragging).not.toHaveBeenCalled();
  });

  it("ignores non-left buttons", () => {
    down($("bar"), 100, 5, 1, 2);
    move(200, 100);
    expect(host.startDragging).not.toHaveBeenCalled();
  });

  it("suppresses the default only on a drag region", () => {
    expect(down($("bar"), 100, 5).defaultPrevented).toBe(true);
    expect(down($("outside"), 100, 5).defaultPrevented).toBe(false);
  });
});

describe("double click", () => {
  it("toggles maximize instead of dragging", () => {
    down($("bar"), 100, 5, 2);
    move(200, 100);
    expect(host.toggleMaximize).toHaveBeenCalledTimes(1);
    expect(host.startDragging).not.toHaveBeenCalled();
  });

  it("ignores a triple click", () => {
    down($("bar"), 100, 5, 3);
    expect(host.toggleMaximize).not.toHaveBeenCalled();
  });
});
