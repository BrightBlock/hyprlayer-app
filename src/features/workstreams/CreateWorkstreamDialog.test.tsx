// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { CreateWorkstreamDialog } from "./CreateWorkstreamDialog";
import type { WorkstreamBases } from "@/lib/bindings";

// Mock the data hooks so the dialog runs its real state logic against a plain
// bases object — no react-query, no git subprocess. `vi.hoisted` lets the
// (hoisted) vi.mock factory reach them.
const q = vi.hoisted(() => ({
  useWorkstreamBases: vi.fn(),
  useCreateWorkstream: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);

afterEach(cleanup);

type BasesResult = {
  data: WorkstreamBases;
  isLoading: boolean;
  isError: boolean;
  error: unknown;
  refetch: () => void;
};

function basesResult(data: Partial<WorkstreamBases>, refetch = vi.fn()): BasesResult {
  return {
    data: {
      branches: [],
      localBranches: [],
      defaultBase: "",
      currentBranch: "",
      ...data,
    },
    isLoading: false,
    isError: false,
    error: null,
    refetch,
  };
}

function mockBases(data: Partial<WorkstreamBases>, refetch = vi.fn()) {
  q.useWorkstreamBases.mockReturnValue(basesResult(data, refetch));
}

// Stable no-op mutation.
q.useCreateWorkstream.mockReturnValue({
  isPending: false,
  isError: false,
  error: null,
  reset: vi.fn(),
  mutate: vi.fn(),
});

function renderDialog(open: boolean) {
  return render(
    <CreateWorkstreamDialog
      projectPath="/repo"
      open={open}
      onClose={vi.fn()}
      onCreated={vi.fn()}
    />,
  );
}

// The "Branched from" base picker — the only <select> in the dialog.
const baseSelect = () => screen.getByRole("combobox") as HTMLSelectElement;

describe("CreateWorkstreamDialog base detection", () => {
  it("re-scans local branches on the open edge, not while closed", () => {
    const refetch = vi.fn();
    mockBases({ branches: ["dev"], localBranches: ["dev"], defaultBase: "dev" }, refetch);

    const { rerender } = renderDialog(false);
    expect(refetch).not.toHaveBeenCalled();

    rerender(
      <CreateWorkstreamDialog
        projectPath="/repo"
        open
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("adopts a main branch that only appears on the open refetch", () => {
    // First paint: repo has no main — the base falls back to the first branch.
    mockBases({
      branches: ["feature-x"],
      localBranches: ["feature-x"],
      defaultBase: "",
      currentBranch: "feature-x",
    });
    const { rerender } = renderDialog(true);
    expect(baseSelect().value).toBe("feature-x");

    // The open refetch lands: `main` now exists on disk and is the detected
    // default. The picker must follow it — the whole point of the fix.
    mockBases({
      branches: ["main", "feature-x"],
      localBranches: ["main", "feature-x"],
      defaultBase: "main",
      currentBranch: "feature-x",
    });
    rerender(
      <CreateWorkstreamDialog
        projectPath="/repo"
        open
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );
    expect(baseSelect().value).toBe("main");
    // And `main` is offered as an option.
    expect(screen.getByRole("option", { name: "main" })).toBeDefined();
  });

  it("keeps a manually chosen base when fresh branches arrive", () => {
    mockBases({
      branches: ["dev", "feature-x"],
      localBranches: ["dev", "feature-x"],
      defaultBase: "dev",
      currentBranch: "feature-x",
    });
    const { rerender } = renderDialog(true);
    expect(baseSelect().value).toBe("dev");

    // User overrides the default.
    fireEvent.change(baseSelect(), { target: { value: "feature-x" } });
    expect(baseSelect().value).toBe("feature-x");

    // A later refetch surfaces `main` as the new default — the manual pick wins.
    mockBases({
      branches: ["main", "dev", "feature-x"],
      localBranches: ["main", "dev", "feature-x"],
      defaultBase: "main",
      currentBranch: "feature-x",
    });
    rerender(
      <CreateWorkstreamDialog
        projectPath="/repo"
        open
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );
    expect(baseSelect().value).toBe("feature-x");
  });
});
