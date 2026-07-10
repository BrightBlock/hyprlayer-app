// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FullDiskAccessNotice } from "./FullDiskAccessNotice";
import type { AppSettings, Bootstrap, FullDiskAccess } from "@/lib/bindings";

const q = vi.hoisted(() => ({
  useFullDiskAccess: vi.fn(),
  useBootstrap: vi.fn(),
  useSetAppSettings: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);

const show = vi.hoisted(() => vi.fn());
vi.mock("@/lib/toast", () => ({ useToast: { getState: () => ({ show }) } }));

const openSettings = vi.hoisted(() => vi.fn(() => Promise.resolve({ status: "ok", data: null })));
vi.mock("@/lib/bindings", () => ({
  commands: { openFullDiskAccessSettings: openSettings },
}));

const mutate = vi.fn();

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function renderNotice(
  access: FullDiskAccess,
  settings: Partial<AppSettings> = { fullDiskAccessNoticeDismissed: false },
) {
  q.useFullDiskAccess.mockReturnValue({ data: access });
  q.useBootstrap.mockReturnValue({
    data: { settings: settings as AppSettings } as Bootstrap,
  });
  q.useSetAppSettings.mockReturnValue({ mutate, isPending: false });
  return render(<FullDiskAccessNotice />);
}

describe("FullDiskAccessNotice", () => {
  it("raises a warning toast when the grant is missing", () => {
    renderNotice("denied");
    expect(show).toHaveBeenCalledTimes(1);
    const [title, , opts] = show.mock.calls[0];
    expect(title).toMatch(/Full Disk Access/);
    expect(opts.tone).toBe("warn");
    expect(opts.action.label).toBe("Open Settings…");
  });

  it("stays silent once the grant is held", () => {
    renderNotice("granted");
    expect(show).not.toHaveBeenCalled();
  });

  /** Non-macOS, or the probe file is absent: never nag. */
  it("stays silent when the probe is inconclusive", () => {
    renderNotice("unknown");
    expect(show).not.toHaveBeenCalled();
  });

  it("stays silent after the user dismisses it", () => {
    renderNotice("denied", { fullDiskAccessNoticeDismissed: true });
    expect(show).not.toHaveBeenCalled();
  });

  it("opens System Settings from the toast action", () => {
    renderNotice("denied");
    void show.mock.calls[0][2].action.onClick();
    expect(openSettings).toHaveBeenCalled();
  });

  it("dismisses against the freshest settings, not a stale snapshot", () => {
    // Toast fires with worktreeRoot "/a"; the user then changes it to "/b"
    // (a re-render) before dismissing. The persisted dismissal must carry "/b".
    const { rerender } = renderNotice("denied", {
      fullDiskAccessNoticeDismissed: false,
      worktreeRoot: "/a",
    } as Partial<AppSettings>);
    const onDismiss = show.mock.calls[0][2].onDismiss;

    q.useBootstrap.mockReturnValue({
      data: {
        settings: {
          fullDiskAccessNoticeDismissed: false,
          worktreeRoot: "/b",
        } as AppSettings,
      } as Bootstrap,
    });
    rerender(<FullDiskAccessNotice />);

    onDismiss();
    expect(mutate).toHaveBeenCalledWith(
      expect.objectContaining({ worktreeRoot: "/b", fullDiskAccessNoticeDismissed: true }),
    );
  });

  it("clears a past dismissal once access is granted, so a later regression re-warns", () => {
    renderNotice("granted", { fullDiskAccessNoticeDismissed: true });
    expect(mutate).toHaveBeenCalledWith(
      expect.objectContaining({ fullDiskAccessNoticeDismissed: false }),
    );
    expect(show).not.toHaveBeenCalled();
  });

  it("does not rewrite settings when granted and no dismissal is stored", () => {
    renderNotice("granted", { fullDiskAccessNoticeDismissed: false });
    expect(mutate).not.toHaveBeenCalled();
  });
});
