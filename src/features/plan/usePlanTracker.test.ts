// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";

import type { SessionEntry } from "@/features/session/sessionStore";
import { initialSessionView, type TranscriptItem } from "@/features/session/transcript";

vi.mock("@/features/session/sessionStore", () => ({ useSession: () => undefined }));
vi.mock("@/lib/bindings", () => ({ events: {}, commands: {} }));

const { sessionSignalOf } = await import("./usePlanTracker");

function entry(items: TranscriptItem[], over: Partial<SessionEntry> = {}): SessionEntry {
  return {
    ...initialSessionView("running"),
    items,
    workstreamId: "ws",
    stage: "implement",
    autoAllow: [],
    live: true,
    permissionMode: null,
    queued: [],
    workingSince: null,
    taskStartedAt: {},
    ...over,
  };
}

const agent = (toolUseId: string, input: Record<string, unknown>, result: { content: string; isError: boolean } | null) =>
  ({ kind: "tool", id: `id-${toolUseId}`, toolUseId, name: "Agent", input: JSON.stringify(input), result, parentId: null }) satisfies TranscriptItem;

describe("sessionSignalOf with a subagent", () => {
  const foreground = agent("f1", { subagent_type: "foreman", run_in_background: false }, null);
  const background = agent("b1", { subagent_type: "cartographer" }, { content: "Async agent launched successfully.", isError: false });

  it("reports a foreground subagent while the turn streams", () => {
    expect(sessionSignalOf(entry([foreground], { streaming: true }))).toMatchObject({
      activity: "working",
      subagent: { toolUseId: "f1" },
    });
  });

  it("keeps a background subagent working after the turn ends", () => {
    expect(sessionSignalOf(entry([background]))).toMatchObject({ activity: "working", subagent: { toolUseId: "b1" } });
  });

  it("drops a foreground subagent left without a result once the session is idle", () => {
    expect(sessionSignalOf(entry([foreground]))).toMatchObject({ activity: "idle", subagent: null });
  });

  it("keeps the subagent while the session waits on a permission inside it", () => {
    const waiting = entry([foreground], {
      state: "awaiting_permission",
      pendingPermission: { requestId: "p1", toolName: "Bash", input: "{}" },
    });
    expect(sessionSignalOf(waiting)).toMatchObject({ activity: "waiting", subagent: { toolUseId: "f1" } });
  });

  it("ignores the subagents of a detached session", () => {
    expect(sessionSignalOf(entry([background], { live: false }))).toMatchObject({ activity: "idle", subagent: null });
  });
});
