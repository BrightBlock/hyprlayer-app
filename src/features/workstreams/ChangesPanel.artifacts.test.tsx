// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ChangesPanel } from "./ChangesPanel";
import type { SessionArtifact } from "@/features/session/artifacts";

// Plain values instead of react-query/IPC, so the panel runs its real logic.
const q = vi.hoisted(() => ({
  useWorktreeGitStatus: vi.fn(),
  useWorktreeDir: vi.fn(),
  useStageFiles: vi.fn(),
  useUnstageFiles: vi.fn(),
  useDiscardFiles: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);
vi.mock("@/lib/queryClient", () => ({ queryClient: { invalidateQueries: vi.fn() } }));

const store = vi.hoisted(() => ({
  useWorkstreamDocs: vi.fn((): unknown[] => []),
  useWorkstreamArtifacts: vi.fn((): SessionArtifact[] => []),
}));
vi.mock("@/features/session/sessionStore", () => store);

const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => opener);

afterEach(cleanup);

function artifact(overrides: Partial<SessionArtifact> = {}): SessionArtifact {
  return {
    url: "https://claude.ai/code/artifact/abc",
    title: "linux and codex rollout",
    description: "Three hero install-row options",
    filePath: "/tmp/out.html",
    ...overrides,
  };
}

function setup() {
  q.useWorktreeGitStatus.mockReturnValue({
    data: { staged: [], unstaged: [] },
    isLoading: false,
    isError: false,
    isFetching: false,
    error: null,
    refetch: vi.fn(),
  });
  q.useWorktreeDir.mockReturnValue({ data: [], isLoading: false, isError: false, error: null });
  const mutation = () => ({ mutate: vi.fn(), isPending: false });
  q.useStageFiles.mockReturnValue(mutation());
  q.useUnstageFiles.mockReturnValue(mutation());
  q.useDiscardFiles.mockReturnValue(mutation());

  render(
    <ChangesPanel workstreamId="ws1" onOpenDoc={vi.fn()} onOpenDiff={vi.fn()} onOpenFile={vi.fn()} />,
  );
}

describe("ChangesPanel artifacts section", () => {
  it("renders the Artifacts heading and each title when artifacts are present", () => {
    store.useWorkstreamArtifacts.mockReturnValue([
      artifact({ url: "a", title: "linux and codex rollout" }),
      artifact({ url: "b", title: "perf dashboard" }),
    ]);
    setup();
    expect(screen.getByText("Artifacts")).toBeDefined();
    expect(screen.getByText("linux and codex rollout")).toBeDefined();
    expect(screen.getByText("perf dashboard")).toBeDefined();
  });

  it("opens the artifact's URL in the browser when a row is clicked", () => {
    store.useWorkstreamArtifacts.mockReturnValue([artifact({ url: "https://claude.ai/code/artifact/abc" })]);
    setup();
    fireEvent.click(screen.getByText("linux and codex rollout"));
    expect(opener.openUrl).toHaveBeenCalledWith("https://claude.ai/code/artifact/abc");
  });

  it("renders neither the heading nor any row when there are no artifacts", () => {
    store.useWorkstreamArtifacts.mockReturnValue([]);
    setup();
    expect(screen.queryByText("Artifacts")).toBeNull();
  });
});
