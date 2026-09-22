// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentUpdateFailed, AgentUpdateReport } from "./AgentBundleDialogs";
import type { BundleFailure, BundleReport } from "./agents";

const b = vi.hoisted(() => ({ navigate: vi.fn() }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => b.navigate }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const report = (over: Partial<BundleReport> = {}): BundleReport => ({
  from: "1.5.9",
  to: "1.6.0",
  source: "releaseAsset",
  stdout: "Downloaded the Claude + Codex bundle pair\n",
  ...over,
});

describe("AgentUpdateReport", () => {
  it("renders nothing when there is no report yet", () => {
    render(<AgentUpdateReport report={null} open={true} onClose={() => {}} />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("reports the atomic pair and its version transition", () => {
    render(<AgentUpdateReport report={report()} open={true} onClose={() => {}} />);

    expect(screen.getByText("1.5.9 → 1.6.0")).toBeTruthy();
    expect(screen.getByText(/Claude Code and Codex now point to 1\.6\.0/)).toBeTruthy();
    expect(screen.queryByText(/backup/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /restore/i })).toBeNull();
  });

  it("warns when the CLI selected the repository fallback", () => {
    render(
      <AgentUpdateReport
        report={report({ source: "fallback", stdout: "trying the next source" })}
        open={true}
        onClose={() => {}}
      />,
    );

    expect(screen.getByText(/repository fallback bundle/)).toBeTruthy();
    expect(screen.getByText(/trying the next source/)).toBeTruthy();
  });

  it("does not invent an identity when the target is unknown", () => {
    render(
      <AgentUpdateReport
        report={report({ from: null, to: null, source: "unknown" })}
        open={true}
        onClose={() => {}}
      />,
    );

    expect(screen.queryByText(/→/)).toBeNull();
    expect(screen.getByText(/did not identify the bundle source/)).toBeTruthy();
  });
});

describe("AgentUpdateFailed", () => {
  const failure: BundleFailure = {
    message: "network error: connection reset",
    stillOn: "1.5.9",
  };

  it("renders nothing when there is no failure yet", () => {
    render(
      <AgentUpdateFailed failure={null} open={true} onClose={() => {}} onRetry={() => {}} />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("leads with what it is most likely still on, and keeps the CLI's error folded away", () => {
    render(
      <AgentUpdateFailed failure={failure} open={true} onClose={() => {}} onRetry={() => {}} />,
    );
    expect(screen.getByText("1.5.9")).toBeTruthy();
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

  it("Open AI settings closes the dialog and routes to settings", () => {
    const onClose = vi.fn();
    render(
      <AgentUpdateFailed failure={failure} open={true} onClose={onClose} onRetry={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open AI settings" }));
    expect(onClose).toHaveBeenCalled();
    expect(b.navigate).toHaveBeenCalledWith({ to: "/settings", search: { section: "general" } });
  });
});
