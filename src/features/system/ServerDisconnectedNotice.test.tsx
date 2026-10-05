// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useToast } from "@/lib/toast";
import { ServerDisconnectedNotice } from "./ServerDisconnectedNotice";

const b = vi.hoisted(() => {
  let listener: (() => void) | null = null;
  return {
    disconnect: () => listener?.(),
    events: {
      serverDisconnected: {
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
vi.mock("@/lib/bindings", () => ({ events: b.events }));
const q = vi.hoisted(() => ({ useConnectionStatus: vi.fn() }));
vi.mock("@/lib/queries", () => q);
const planned = vi.hoisted(() => ({ isPlannedRestart: vi.fn(() => false) }));
vi.mock("@/features/hosts/plannedRestart", () => planned);

function mockState(state: "connected" | "lost", dataUpdatedAt: number) {
  q.useConnectionStatus.mockReturnValue({ data: { state }, dataUpdatedAt });
}

beforeEach(() => {
  mockState("connected", 0);
  useToast.getState().hide();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerDisconnectedNotice", () => {
  it("shows nothing until the server connection ends", () => {
    render(<ServerDisconnectedNotice />);
    expect(useToast.getState().visible).toBe(false);
  });

  it("warns that it is reconnecting, with nothing to press", () => {
    render(<ServerDisconnectedNotice />);
    act(() => b.disconnect());
    const toast = useToast.getState();
    expect(toast.visible).toBe(true);
    expect(toast.tone).toBe("warn");
    expect(toast.title).toContain("Lost connection");
    expect(toast.message).toContain("Reconnecting");
    expect(toast.action).toBeNull();
  });

  it("stays quiet for a restart this app asked for", () => {
    planned.isPlannedRestart.mockReturnValueOnce(true);
    render(<ServerDisconnectedNotice />);
    act(() => b.disconnect());
    expect(useToast.getState().visible).toBe(false);
  });

  it("takes the toast down once the connection is back", () => {
    const { rerender } = render(<ServerDisconnectedNotice />);
    act(() => b.disconnect());
    mockState("lost", 1);
    rerender(<ServerDisconnectedNotice />);
    expect(useToast.getState().visible).toBe(true);

    mockState("connected", 2);
    rerender(<ServerDisconnectedNotice />);
    expect(useToast.getState().visible).toBe(false);
  });

  it("leaves an unrelated toast alone", () => {
    const { rerender } = render(<ServerDisconnectedNotice />);
    act(() => useToast.getState().show("Something else", ""));
    mockState("connected", 1);
    rerender(<ServerDisconnectedNotice />);
    expect(useToast.getState().visible).toBe(true);
  });
});
