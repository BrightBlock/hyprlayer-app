// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useKeybindings } from "./useKeybindings";
import { useCommand, useEscape } from "./registry";

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
        <CommandHost id="nav.settings" handler={fn} />
      </>,
    );
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    expect(fn).toHaveBeenCalledOnce();
  });

  it("does nothing when no handler is registered (wrong context)", () => {
    render(<Dispatcher />);
    const evt = new KeyboardEvent("keydown", { key: ",", ctrlKey: true, cancelable: true });
    window.dispatchEvent(evt);
    expect(evt.defaultPrevented).toBe(false);
  });

  it("routes a command's alternate chord to the same handler", () => {
    const fn = vi.fn();
    render(
      <>
        <Dispatcher />
        <CommandHost id="tab.prev" handler={fn} />
      </>,
    );
    fireEvent.keyDown(window, { key: "{", ctrlKey: true, shiftKey: true });
    expect(fn).toHaveBeenCalledOnce();
    fireEvent.keyDown(window, { key: "PageUp", ctrlKey: true });
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("ignores a bare-key chord while typing in an input but still fires mod chords", () => {
    const bare = vi.fn();
    const modded = vi.fn();
    const { container } = render(
      <>
        <Dispatcher overrides={{ "tab.new": "n" }} />
        <CommandHost id="tab.new" handler={bare} />
        <CommandHost id="nav.settings" handler={modded} />
        <input data-testid="field" />
      </>,
    );
    const input = container.querySelector("input")!;
    input.focus();

    fireEvent.keyDown(input, { key: "n" });
    expect(bare).not.toHaveBeenCalled();

    fireEvent.keyDown(input, { key: ",", ctrlKey: true });
    expect(modded).toHaveBeenCalledOnce();
  });

  it("fires a bare-key chord while a switch has focus, since a checkbox takes no text", () => {
    const bare = vi.fn();
    const { container } = render(
      <>
        <Dispatcher overrides={{ "tab.new": "n" }} />
        <CommandHost id="tab.new" handler={bare} />
        <input type="checkbox" role="switch" aria-label="Notify" />
      </>,
    );
    const box = container.querySelector("input")!;
    box.focus();
    fireEvent.keyDown(box, { key: "n" });
    expect(bare).toHaveBeenCalledOnce();
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
