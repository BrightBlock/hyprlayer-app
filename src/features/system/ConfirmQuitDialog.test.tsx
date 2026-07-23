// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ConfirmQuitDialog } from "./ConfirmQuitDialog";

const b = vi.hoisted(() => {
  let listener: (() => void) | null = null;
  return {
    /** Fire the backend's quit-requested event at the mounted component. */
    requestQuit: () => listener?.(),
    quitApp: vi.fn(() => Promise.resolve({ status: "ok", data: null })),
    events: {
      quitRequested: {
        listen: (cb: () => void) => {
          listener = cb;
          return Promise.resolve(() => {
            listener = null;
          });
        },
      },
    },
  };
});
vi.mock("@/lib/bindings", () => ({ commands: { quitApp: b.quitApp }, events: b.events }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/** Mount, then raise the event a ⌘Q / window-close produces. */
function openPrompt() {
  const view = render(<ConfirmQuitDialog />);
  act(() => b.requestQuit());
  return view;
}

const quitButton = () => screen.getByRole("button", { name: "Quit" });
const checkbox = () => screen.getByRole("checkbox") as HTMLInputElement;

describe("ConfirmQuitDialog", () => {
  it("stays out of the way until a quit is requested", () => {
    render(<ConfirmQuitDialog />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("prompts when the backend requests a quit", () => {
    openPrompt();
    expect(screen.getByText("Quit Hyprlayer?")).toBeTruthy();
  });

  it("quits without touching the setting when confirmed", () => {
    openPrompt();
    fireEvent.click(quitButton());
    expect(b.quitApp).toHaveBeenCalledWith(false);
  });

  it("persists the opt-out along with the confirmation", () => {
    openPrompt();
    fireEvent.click(checkbox());
    fireEvent.click(quitButton());
    expect(b.quitApp).toHaveBeenCalledWith(true);
  });

  // The opt-out rides with the confirmation: cancelling leaves the setting
  // alone, and the backend has already cancelled the exit itself.
  it("does nothing on cancel, even with the box checked", () => {
    openPrompt();
    fireEvent.click(checkbox());
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(b.quitApp).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("starts each prompt unchecked", () => {
    openPrompt();
    fireEvent.click(checkbox());
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    act(() => b.requestQuit());
    expect(checkbox().checked).toBe(false);
  });

  // The command only returns if the exit did NOT happen (the settings write
  // failed), so the dialog has to recover rather than hang on a spinner.
  it("surfaces a failed quit and re-arms the button", async () => {
    b.quitApp.mockResolvedValueOnce({ status: "error", error: "disk full" } as never);
    openPrompt();
    fireEvent.click(quitButton());

    expect(await screen.findByText("disk full")).toBeTruthy();
    expect(quitButton().hasAttribute("disabled")).toBe(false);
  });
});
