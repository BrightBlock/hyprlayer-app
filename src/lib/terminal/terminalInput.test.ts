// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

// Ways a press could fail to take a shared terminal's size back:
// - xterm stops the press before it reaches the listener, as it does while the program tracks the mouse;
// - a press before the terminal has an id calls the server with none;
// - a press after unmount still claims.

const terminalFocus = vi.fn();
vi.mock("@/lib/bindings", () => ({ commands: { terminalFocus } }));

const { claimOnPress } = await import("./terminalInput");

function press(el: HTMLElement) {
  el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
}

beforeEach(() => terminalFocus.mockClear());

describe("claimOnPress", () => {
  it("claims even when the terminal keeps the press to itself", () => {
    const host = document.createElement("div");
    const screen = document.createElement("div");
    host.append(screen);
    screen.addEventListener("mousedown", (e) => e.stopPropagation());
    claimOnPress(host, () => "t1");
    press(screen);
    expect(terminalFocus).toHaveBeenCalledWith("t1");
  });

  it("does nothing before the terminal has an id", () => {
    const host = document.createElement("div");
    claimOnPress(host, () => undefined);
    press(host);
    expect(terminalFocus).not.toHaveBeenCalled();
  });

  it("stops claiming once unsubscribed", () => {
    const host = document.createElement("div");
    const off = claimOnPress(host, () => "t1");
    off();
    press(host);
    expect(terminalFocus).not.toHaveBeenCalled();
  });
});
