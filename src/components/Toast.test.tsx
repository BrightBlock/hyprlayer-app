// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Toast } from "./Toast";
import { useToast } from "@/lib/toast";

const DURATION_MS = 5000;
const EXIT_MS = 380;

function reset() {
  useToast.setState({ visible: false, title: "", message: "", tone: "ok", action: null, onDismiss: null });
}

beforeEach(() => {
  vi.useFakeTimers();
  reset();
});

afterEach(() => {
  reset();
  cleanup();
  vi.useRealTimers();
});

describe("Toast", () => {
  it("renders nothing until shown", () => {
    render(<Toast />);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("shows title and message", () => {
    render(<Toast />);
    act(() => useToast.getState().show("Hello", "World"));
    expect(screen.getByText("Hello")).toBeTruthy();
    expect(screen.getByText("World")).toBeTruthy();
  });

  it("a plain toast shows a countdown and auto-dismisses", () => {
    render(<Toast />);
    act(() => useToast.getState().show("Up to date", "Latest version"));
    expect(screen.getByText(/^\d+s$/)).toBeTruthy();
    expect(useToast.getState().visible).toBe(true);
    act(() => vi.advanceTimersByTime(DURATION_MS));
    expect(useToast.getState().visible).toBe(false);
    act(() => vi.advanceTimersByTime(EXIT_MS));
    expect(screen.queryByText("Latest version")).toBeNull();
  });

  it("an action toast is sticky: shows the button, no countdown, and never auto-dismisses", () => {
    const onClick = vi.fn();
    render(<Toast />);
    act(() =>
      useToast.getState().show("Update available", "v9.9.9", {
        action: { label: "Restart to update", onClick },
      }),
    );
    expect(screen.getByText("Restart to update")).toBeTruthy();
    expect(screen.queryByText(/^\d+s$/)).toBeNull();
    act(() => vi.advanceTimersByTime(DURATION_MS * 3));
    expect(screen.getByText("v9.9.9")).toBeTruthy();
  });

  it("clicking the action invokes its handler", () => {
    const onClick = vi.fn();
    render(<Toast />);
    act(() =>
      useToast.getState().show("Update available", "v9.9.9", {
        action: { label: "Restart to update", onClick },
      }),
    );
    act(() => fireEvent.click(screen.getByText("Restart to update")));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("dismiss (×) hides the toast and fires onDismiss", () => {
    const onDismiss = vi.fn();
    render(<Toast />);
    act(() =>
      useToast.getState().show("Update available", "v9.9.9", {
        action: { label: "Restart to update", onClick: vi.fn() },
        onDismiss,
      }),
    );
    act(() => fireEvent.click(screen.getByLabelText("Dismiss")));
    expect(onDismiss).toHaveBeenCalledOnce();
    expect(useToast.getState().visible).toBe(false);
  });
});
