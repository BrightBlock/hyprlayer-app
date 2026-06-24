// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useKeybindings } from "./useKeybindings";
import { useCommand, useEscape } from "./registry";

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

function EscapeHost({ handler }: { handler: (() => void) | null }) {
  useEscape(handler);
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

describe("useKeybindings Escape routing", () => {
  it("routes bare Escape to the registered Escape handler and consumes the key", () => {
    const onEscape = vi.fn();
    render(
      <>
        <Dispatcher />
        <EscapeHost handler={onEscape} />
      </>,
    );
    const evt = new KeyboardEvent("keydown", { key: "Escape", cancelable: true });
    window.dispatchEvent(evt);
    expect(onEscape).toHaveBeenCalledOnce();
    expect(evt.defaultPrevented).toBe(true);
  });

  it("leaves Escape untouched when nothing is registered (terminal/editor keeps it)", () => {
    render(<Dispatcher />);
    const evt = new KeyboardEvent("keydown", { key: "Escape", cancelable: true });
    window.dispatchEvent(evt);
    expect(evt.defaultPrevented).toBe(false);
  });

  it("closes the topmost surface first: a modal over a page shadows the page", () => {
    const page = vi.fn();
    const modal = vi.fn();
    const { rerender } = render(
      <>
        <Dispatcher />
        <EscapeHost handler={page} />
      </>,
    );
    // Modal opens on top of the page — it's the latest registrant, so it wins.
    rerender(
      <>
        <Dispatcher />
        <EscapeHost handler={page} />
        <EscapeHost handler={modal} />
      </>,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(modal).toHaveBeenCalledOnce();
    expect(page).not.toHaveBeenCalled();

    // Modal closes; the next Escape falls through to the page beneath it.
    rerender(
      <>
        <Dispatcher />
        <EscapeHost handler={page} />
      </>,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(page).toHaveBeenCalledOnce();
  });

  it("fires Escape even while focus is in an input (close a dialog you're typing in)", () => {
    const onEscape = vi.fn();
    const { container } = render(
      <>
        <Dispatcher />
        <EscapeHost handler={onEscape} />
        <input data-testid="field" />
      </>,
    );
    const input = container.querySelector("input")!;
    input.focus();
    fireEvent.keyDown(input, { key: "Escape" });
    expect(onEscape).toHaveBeenCalledOnce();
  });

  it("does not route a modified Escape (e.g. the reserved ⌘⇧Esc) to the Escape stack", () => {
    const onEscape = vi.fn();
    render(
      <>
        <Dispatcher />
        <EscapeHost handler={onEscape} />
      </>,
    );
    fireEvent.keyDown(window, { key: "Escape", ctrlKey: true, shiftKey: true });
    expect(onEscape).not.toHaveBeenCalled();
  });
});
