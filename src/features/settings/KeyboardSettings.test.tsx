// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { KeyboardSettings } from "./KeyboardSettings";
import type { AppSettings } from "../../lib/bindings";

const mutate = vi.fn();

vi.mock("../../lib/queries", () => ({
  useSetAppSettings: () => ({ mutate, isError: false, error: null }),
}));

afterEach(() => {
  cleanup();
  mutate.mockReset();
});

const SETTINGS: AppSettings = { keybindings: { overrides: {} } };

/** The row container for a command, located via its visible label. */
function rowFor(label: string): HTMLElement {
  return screen.getByText(label).closest("div.flex.items-start") as HTMLElement;
}

describe("KeyboardSettings", () => {
  it("renders commands grouped by category", () => {
    render(<KeyboardSettings settings={SETTINGS} />);
    expect(screen.getByText("Navigation")).toBeTruthy();
    expect(screen.getByText("Go to Dashboard")).toBeTruthy();
    // jsdom is the non-mac branch, so chords render with Ctrl labels.
    expect(screen.getAllByText("Ctrl+1").length).toBeGreaterThan(0);
  });

  it("records a new chord and persists it on Save", () => {
    render(<KeyboardSettings settings={SETTINGS} />);
    const row = rowFor("Go to Dashboard");

    fireEvent.click(within(row).getByText("Edit"));
    // Capture-phase listener is on window; non-mac maps Ctrl → mod.
    fireEvent.keyDown(window, { key: "9", ctrlKey: true });
    // The candidate is shown before committing.
    expect(within(row).getByText("Ctrl+9")).toBeTruthy();

    fireEvent.click(within(row).getByText("Save"));
    expect(mutate).toHaveBeenCalledTimes(1);
    expect(mutate).toHaveBeenCalledWith({
      keybindings: { overrides: { "nav.dashboard": "mod+9" } },
    });
  });

  it("warns when a recorded chord conflicts with another command", () => {
    render(<KeyboardSettings settings={SETTINGS} />);
    const row = rowFor("Go to Library");

    fireEvent.click(within(row).getByText("Edit"));
    // Ctrl+1 is Go to Dashboard's default — recording it here conflicts.
    fireEvent.keyDown(window, { key: "1", ctrlKey: true });
    expect(within(row).getByText(/Already used by Go to Dashboard/)).toBeTruthy();
  });

  it("unbinds a command (null override) via Unbind", () => {
    render(<KeyboardSettings settings={SETTINGS} />);
    const row = rowFor("Go to Dashboard");
    fireEvent.click(within(row).getByText("Unbind"));
    expect(mutate).toHaveBeenCalledWith({
      keybindings: { overrides: { "nav.dashboard": null } },
    });
  });

  it("resets a command by dropping its override key", () => {
    const withOverride: AppSettings = {
      keybindings: { overrides: { "nav.dashboard": "mod+9" } },
    };
    render(<KeyboardSettings settings={withOverride} />);
    const row = rowFor("Go to Dashboard");
    fireEvent.click(within(row).getByText("Reset"));
    expect(mutate).toHaveBeenCalledWith({ keybindings: { overrides: {} } });
  });
});
