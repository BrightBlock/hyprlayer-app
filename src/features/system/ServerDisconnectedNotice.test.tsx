// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppError } from "@/lib/bindings";
import { useToast } from "@/lib/toast";
import { useReconnectStore } from "@/features/hosts/reconnectState";
import { useServerAutoUpdate } from "@/features/hosts/serverAutoUpdate";
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
vi.mock("@/features/hosts/serverAutoUpdate", async () => {
  const { create } = await import("zustand");
  return { useServerAutoUpdate: create(() => ({ enabled: true, updating: false })) };
});

const KEY_CHANGED: AppError = {
  code: "remote_host_key_changed",
  message: "The host key for Nexus has changed.",
  remedy: "If you expected it, remove the old key with `ssh-keygen -R nexus` and connect again.",
};

function mockState(state: "connected" | "lost", dataUpdatedAt: number) {
  q.useConnectionStatus.mockReturnValue({ data: { state }, dataUpdatedAt });
}

beforeEach(() => {
  mockState("connected", 0);
  useToast.getState().hide();
  useReconnectStore.setState({ lostAt: null, attempting: false, halted: null, keyChanged: null });
  useServerAutoUpdate.setState({ updating: false });
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

  it("names a changed host key in a bad toast, since no dialog opens to say so", () => {
    mockState("lost", 1);
    render(<ServerDisconnectedNotice />);
    act(() => useReconnectStore.setState({ keyChanged: KEY_CHANGED }));
    const toast = useToast.getState();
    expect(toast).toMatchObject({ visible: true, tone: "bad", title: KEY_CHANGED.message });
    expect(toast.message).toBe(KEY_CHANGED.remedy);
  });

  it("shows a changed key once, not on every retry, and takes it down once reconnected", () => {
    mockState("lost", 1);
    const { rerender } = render(<ServerDisconnectedNotice />);
    act(() => useReconnectStore.setState({ keyChanged: KEY_CHANGED }));
    act(() => useToast.getState().hide());
    act(() => useReconnectStore.setState({ keyChanged: { ...KEY_CHANGED } }));
    expect(useToast.getState().visible).toBe(false);

    act(() => useReconnectStore.setState({ keyChanged: null }));
    act(() => useReconnectStore.setState({ keyChanged: KEY_CHANGED }));
    expect(useToast.getState().visible).toBe(true);
    mockState("connected", 2);
    rerender(<ServerDisconnectedNotice />);
    expect(useToast.getState().visible).toBe(false);
  });

  it("warns once an update's server has not come back", () => {
    useServerAutoUpdate.setState({ updating: true });
    mockState("lost", 1);
    render(<ServerDisconnectedNotice />);
    expect(useToast.getState().visible).toBe(false);

    act(() => useServerAutoUpdate.setState({ updating: false }));
    expect(useToast.getState()).toMatchObject({ visible: true, tone: "warn" });
    expect(useToast.getState().title).toContain("Lost connection");
  });

  it("stays quiet when an update reconnects", () => {
    useServerAutoUpdate.setState({ updating: true });
    render(<ServerDisconnectedNotice />);
    act(() => useServerAutoUpdate.setState({ updating: false }));
    expect(useToast.getState().visible).toBe(false);
  });

  it("leaves a halted loop's own toast up when an update ends on it", () => {
    useServerAutoUpdate.setState({ updating: true });
    mockState("lost", 1);
    render(<ServerDisconnectedNotice />);
    act(() => {
      useReconnectStore.setState({ halted: "protocol" });
      useToast.getState().show("The server speaks protocol 2; this app speaks 1.", "", { tone: "bad" });
    });
    act(() => useServerAutoUpdate.setState({ updating: false }));
    expect(useToast.getState().title).toBe("The server speaks protocol 2; this app speaks 1.");
  });
});
