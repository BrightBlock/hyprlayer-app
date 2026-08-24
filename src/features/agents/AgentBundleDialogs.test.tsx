// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AgentBundleChange } from "@/lib/bindings";

import { AgentUpdateFailed, AgentUpdateReport } from "./AgentBundleDialogs";
import type { BundleFailure, BundleReport } from "./agents";

const b = vi.hoisted(() => ({
  restoreAgentFromBackup: vi.fn(() => Promise.resolve({ status: "ok", data: null })),
  navigate: vi.fn(),
  invalidateQueries: vi.fn(() => Promise.resolve()),
}));
vi.mock("@/lib/bindings", () => ({
  commands: { restoreAgentFromBackup: b.restoreAgentFromBackup },
}));

// The failure dialog's "Open AI settings" goes through the router library's
// useNavigate; jsdom has no router, so stand in for it and assert the target.
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => b.navigate }));
// A restore has to invalidate `agent-files`, or the list keeps serving the
// bundle's copy and the next save writes it back over the restored file.
vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: b.invalidateQueries }),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const change = (over: Partial<AgentBundleChange>): AgentBundleChange => ({
  name: "cartographer",
  change: "added",
  path: "/home/u/.claude/agents/cartographer.md",
  ...over,
});

const BACKUP = "/home/u/.claude/hyprlayer-backup";

/** A report with the backend's own fields defaulted, so each case names only
 *  what it is about. */
const report = (over: Partial<BundleReport>): BundleReport => ({
  fromSha: "abc",
  toSha: "def",
  agents: [],
  settingsChanged: false,
  skillsChanged: 0,
  backupPath: BACKUP,
  ...over,
});

describe("AgentUpdateReport", () => {
  it("renders nothing when there is no report yet", () => {
    render(<AgentUpdateReport report={null} open={true} onClose={() => {}} />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("renders one row per change with the right badge tone", () => {
    render(
      <AgentUpdateReport
        report={report({
          fromSha: "4c5cf370aed5fffee3b98f741bd6d911965c9033",
          toSha: "9a1c2f0aed5fffee3b98f741bd6d911965c9033",
          agents: [
            change({ name: "cartographer", change: "added" }),
            change({
              name: "foreman",
              change: "updated",
              path: "/home/u/.claude/agents/foreman.md",
            }),
          ],
        })}
        open={true}
        onClose={() => {}}
      />,
    );

    expect(screen.getByText("cartographer")).toBeTruthy();
    expect(screen.getByText("New")).toBeTruthy();
    expect(screen.getByText("foreman")).toBeTruthy();
    expect(screen.getByText("Updated")).toBeTruthy();
    // The SHA pair on the header line.
    expect(screen.getByText("4c5cf37 → 9a1c2f0")).toBeTruthy();
  });

  // The backend cannot tell a reverted local edit apart from a genuine
  // upstream change — both are `updated` — so the restore has to be offered
  // on every non-`added` row. Gating it on a "this was yours" label the
  // backend never emits made the whole restore path unreachable.
  it("offers Restore mine on every overwritten row, not on newly added ones", () => {
    render(
      <AgentUpdateReport
        report={report({
          agents: [
            change({ name: "cartographer", change: "added" }),
            change({
              name: "foreman",
              change: "updated",
              path: "/home/u/.claude/agents/foreman.md",
            }),
          ],
        })}
        open={true}
        onClose={() => {}}
      />,
    );

    expect(screen.getAllByRole("button", { name: "Restore mine" })).toHaveLength(1);
    expect(screen.getByText(BACKUP)).toBeTruthy();
  });

  it("omits the backup note when everything was newly added, nothing was overwritten", () => {
    render(
      <AgentUpdateReport
        report={report({ agents: [change({ name: "cartographer", change: "added" })] })}
        open={true}
        onClose={() => {}}
      />,
    );
    expect(screen.queryByText(BACKUP)).toBeNull();
  });

  // `hyprlayer ai reinstall` syncs the whole `claude/` tree, so it replaces
  // the user's global settings.json — permissions, env, hooks — and their
  // skills. An agents-only report would let that happen silently.
  it("names a replaced settings.json and replaced skills, and still offers the backup", () => {
    render(
      <AgentUpdateReport
        report={report({ settingsChanged: true, skillsChanged: 3 })}
        open={true}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText(/settings\.json and 3 skill files/)).toBeTruthy();
    expect(screen.getByText(BACKUP)).toBeTruthy();
    expect(screen.queryByText(/already matched the bundle/)).toBeNull();
  });

  // A retry fired after the freshness query itself stopped answering has no
  // SHA pair to report. Printing a sentinel where a SHA belongs is worse than
  // printing nothing.
  it("omits the SHA line entirely when the landed SHA is unknown", () => {
    render(
      <AgentUpdateReport
        report={report({ fromSha: null, toSha: null, settingsChanged: true })}
        open={true}
        onClose={() => {}}
      />,
    );
    expect(screen.queryByText(/unknown/)).toBeNull();
    expect(screen.getByText(/settings\.json/)).toBeTruthy();
  });

  it("reports an update that changed nothing rather than an empty list", () => {
    render(<AgentUpdateReport report={report({ toSha: "abc" })} open={true} onClose={() => {}} />);
    expect(screen.getByText(/already matched the bundle/)).toBeTruthy();
  });

  it("restoring calls the backend with the row's name and refreshes the library", async () => {
    render(
      <AgentUpdateReport
        report={report({ agents: [change({ name: "cartographer", change: "updated" })] })}
        open={true}
        onClose={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Restore mine" }));
    expect(b.restoreAgentFromBackup).toHaveBeenCalledWith("cartographer");
    await vi.waitFor(() =>
      expect(b.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["agent-files"] }),
    );
  });
});

describe("AgentUpdateFailed", () => {
  const failure: BundleFailure = {
    message: "network error: connection reset",
    stillOnSha: "4c5cf370aed5fffee3b98f741bd6d911965c9033",
  };

  it("renders nothing when there is no failure yet", () => {
    render(
      <AgentUpdateFailed failure={null} open={true} onClose={() => {}} onRetry={() => {}} />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("leads with the SHA it is most likely still on, and keeps the CLI's error folded away", () => {
    render(
      <AgentUpdateFailed failure={failure} open={true} onClose={() => {}} onRetry={() => {}} />,
    );
    expect(screen.getByText("4c5cf37")).toBeTruthy();
    // Present for support, but folded away rather than the headline.
    expect(screen.getByText("Technical details")).toBeTruthy();
    const details = screen.getByText("network error: connection reset").closest("details");
    expect(details).toBeTruthy();
    expect((details as HTMLDetailsElement).open).toBe(false);
  });

  it("Try again calls onRetry", () => {
    const onRetry = vi.fn();
    render(
      <AgentUpdateFailed failure={failure} open={true} onClose={() => {}} onRetry={onRetry} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalled();
  });

  // The fix for "no AI tool configured" is the Settings card that owns
  // Reconfigure, not a terminal the user has to read CLI output in.
  it("Open AI settings closes the dialog and routes to the AI Agents card", () => {
    const onClose = vi.fn();
    render(
      <AgentUpdateFailed failure={failure} open={true} onClose={onClose} onRetry={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open AI settings" }));
    expect(onClose).toHaveBeenCalled();
    expect(b.navigate).toHaveBeenCalledWith({ to: "/settings", search: { section: "general" } });
  });

  it("offers no terminal escape hatch", () => {
    render(
      <AgentUpdateFailed failure={failure} open={true} onClose={() => {}} onRetry={() => {}} />,
    );
    expect(screen.queryByRole("button", { name: "Show output" })).toBeNull();
  });
});
