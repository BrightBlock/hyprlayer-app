// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useKeybindings } from "./useKeybindings";
import { useCommand } from "./registry";

// IS_MACOS derives from the jsdom user agent (not "Macintosh"), so these tests
// run in the non-mac branch where Ctrl maps to `mod`.
afterEach(cleanup);

function Dispatcher({ overrides = {} }: { overrides?: Record<string, string | null> }) {
  useKeybindings(overrides);
  return null;
}

function CommandHost({ id, handler }: { id: string; handler: (() => void) | null }) {
  useCommand(id, handler);
  return null;
}

describe("useKeybindings dispatcher", () => {
  it("routes a chord to the registered command handler", () => {
    const fn = vi.fn();
    render(
      <>
        <Dispatcher />
        <CommandHost id="nav.dashboard" handler={fn} />
      </>,
    );
    fireEvent.keyDown(window, { key: "1", ctrlKey: true });
    expect(fn).toHaveBeenCalledOnce();
  });

  it("does nothing when no handler is registered (wrong context)", () => {
    render(<Dispatcher />);
    // No throw, and the default isn't prevented since nothing ran.
    const evt = new KeyboardEvent("keydown", { key: "1", ctrlKey: true, cancelable: true });
    window.dispatchEvent(evt);
    expect(evt.defaultPrevented).toBe(false);
  });

  it("ignores a bare-key chord while typing in an input but still fires mod chords", () => {
    const bare = vi.fn();
    const modded = vi.fn();
    const { container } = render(
      <>
        <Dispatcher overrides={{ "tab.new": "n" }} />
        <CommandHost id="tab.new" handler={bare} />
        <CommandHost id="nav.dashboard" handler={modded} />
        <input data-testid="field" />
      </>,
    );
    const input = container.querySelector("input")!;
    input.focus();

    // Bare "n" while the input is focused must NOT trigger the command.
    fireEvent.keyDown(input, { key: "n" });
    expect(bare).not.toHaveBeenCalled();

    // A primary-modifier chord still works mid-typing.
    fireEvent.keyDown(input, { key: "1", ctrlKey: true });
    expect(modded).toHaveBeenCalledOnce();
  });

  it("fires a bare-key chord when focus is not in an editable element", () => {
    const bare = vi.fn();
    render(
      <>
        <Dispatcher overrides={{ "tab.new": "n" }} />
        <CommandHost id="tab.new" handler={bare} />
      </>,
    );
    fireEvent.keyDown(window, { key: "n" });
    expect(bare).toHaveBeenCalledOnce();
  });
});
