// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { MutationObserver as QueryMutationObserver } from "@tanstack/react-query";
import { afterEach, describe, expect, it } from "vitest";

import type { LeadView, Project, WorkstreamView } from "@/lib/bindings";
import { queryClient } from "@/lib/queryClient";
import { useUiStore } from "@/lib/uiStore";
import type { Workstream } from "@/lib/bindings";
import {
  agentSpawned,
  CLOCK_SKEW_MS,
  leadAnswered,
  leadExpanded,
  taskAgentSession,
  leadOpened,
  MESSAGE_LEAD_KEY,
  projectAdded,
  taskWorkstream,
  useSignal,
  type SignalInputs,
} from "./signals";

const project = (path: string) => ({ path }) as Project;
const ws = (id: string) => ({ item: { id } }) as WorkstreamView;
const lead = (driving: string[]): LeadView => ({
  projectPath: "/p",
  workstreamId: "main",
  sessionId: "lead",
  driving,
  liveChildren: 0,
  projectRules: null,
  remoteControl: false,
  remoteControlUrl: null,
});

const inputs = (over: Partial<SignalInputs> = {}): SignalInputs => ({
  projectPath: "/p",
  projects: [project("/p")],
  lead: lead([]),
  workstreams: [],
  leadAsking: false,
  tourWorkstreamId: null,
  onBoard: false,
  leadExpanded: false,
  ...over,
});

afterEach(() => queryClient.clear());

describe("signal rules", () => {
  it("projectAdded names the new project", () => {
    expect(projectAdded([], [project("/a")])).toEqual({ done: true, project: "/a" });
    expect(projectAdded(["/a"], [project("/a")]).done).toBe(false);
  });

  it("agentSpawned takes a newly driven workstream, never the Lead's own", () => {
    expect(agentSpawned([], lead(["main"]), [ws("main")]).done).toBe(false);
    expect(agentSpawned([], lead(["w1"]), [])).toEqual({ done: true, workstream: "w1" });
    expect(agentSpawned(["w0"], lead(["w0"]), [ws("w0"), ws("w2")])).toEqual({ done: true, workstream: "w2" });
  });

  it("agentSpawned is done once the tour knows its workstream, even one in the baseline or already archived", () => {
    expect(agentSpawned(["w1"], lead(["w1"]), [ws("w1")], "w1")).toEqual({ done: true, workstream: "w1" });
    expect(agentSpawned([], lead([]), [], "gone")).toEqual({ done: true, workstream: "gone" });
  });

  it("taskWorkstream takes the Lead's first child made after the send, live or archived", () => {
    const item = (id: string, createdAt: number | null, parent: string | null = "main") =>
      ({ id, createdAt, parentWorkstreamId: parent }) as Workstream;
    const sent = 10 * CLOCK_SKEW_MS;
    expect(taskWorkstream(sent, null, [item("w1", sent + 5)])).toBeNull();
    expect(taskWorkstream(sent, "main", [item("main", sent + 1, null)])).toBeNull();
    expect(taskWorkstream(sent, "main", [item("mine", sent + 5, null)])).toBeNull();
    expect(taskWorkstream(sent, "main", [item("old", sent - CLOCK_SKEW_MS - 1)])).toBeNull();
    expect(taskWorkstream(sent, "main", [item("later", sent + 9), item("first", sent + 2)])).toBe("first");
    expect(taskWorkstream(sent, "main", [item("skewed", sent - 1000)])).toBe("skewed");
    // An earlier run's workstream made inside the skew window, or one already there at the send, isn't this task's.
    expect(taskWorkstream(sent, "main", [item("last-run", sent - 60_000)], ["last-run"])).toBeNull();
    expect(taskWorkstream(sent, "main", [item("last-run", sent - 60_000), item("new", sent + 5)], ["last-run"])).toBe("new");
  });

  it("taskAgentSession takes the Lead's newest child started after the send, in any workstream", () => {
    const sent = 10 * CLOCK_SKEW_MS;
    const s = (id: string, parent: string | null, startedAt: number | null, workstreamId = "main") =>
      ({ id, parentSessionId: parent, startedAt, workstreamId }) as never;
    expect(taskAgentSession(sent, null, [s("a", "lead", sent + 1)])).toBeNull();
    expect(taskAgentSession(sent, "lead", [s("other", "someone", sent + 1)])).toBeNull();
    expect(taskAgentSession(sent, "lead", [s("old", "lead", sent - CLOCK_SKEW_MS - 1)])).toBeNull();
    expect(
      taskAgentSession(sent, "lead", [s("first", "lead", sent + 1, "ws-a"), s("newest", "lead", sent + 9, "main")])?.id,
    ).toBe("newest");
  });

  it("leadOpened takes a click on the project's row, or arriving at its board", () => {
    expect(leadOpened(false, false, false).done).toBe(false);
    expect(leadOpened(true, true, false).done).toBe(false);
    expect(leadOpened(true, true, true).done).toBe(true);
    expect(leadOpened(false, true, false).done).toBe(true);
  });

  it("leadExpanded needs the dock collapsed at some point first", () => {
    expect(leadExpanded(false, true).done).toBe(false);
    expect(leadExpanded(true, true).done).toBe(true);
    expect(leadExpanded(true, false).done).toBe(false);
  });

  it("leadAnswered fires when the question clears, or the tour's workstream is archived", () => {
    expect(leadAnswered(true, false, null, []).done).toBe(true);
    expect(leadAnswered(false, false, null, []).done).toBe(false);
    expect(leadAnswered(true, true, null, []).done).toBe(false);
    expect(leadAnswered(false, false, "w1", [ws("w2")]).done).toBe(true);
    expect(leadAnswered(false, false, "w1", [ws("w1")]).done).toBe(false);
  });
});

describe("useSignal", () => {
  it("projectAdded measures against the projects at step start", () => {
    const { result, rerender } = renderHook((p: SignalInputs) => useSignal("projectAdded", p), {
      initialProps: inputs({ projects: [] }),
    });
    expect(result.current.done).toBe(false);
    rerender(inputs({ projects: [project("/new")] }));
    expect(result.current).toEqual({ done: true, project: "/new" });
  });

  it("agentSpawned ignores workstreams that were already there", () => {
    const { result, rerender } = renderHook((p: SignalInputs) => useSignal("agentSpawned", p), {
      initialProps: inputs({ workstreams: [ws("old")] }),
    });
    expect(result.current.done).toBe(false);
    rerender(inputs({ workstreams: [ws("old"), ws("tour")] }));
    expect(result.current).toEqual({ done: true, workstream: "tour" });
  });

  it("leadMessageSent fires on the message-lead mutation's success for this project", async () => {
    const { result } = renderHook(() => useSignal("leadMessageSent", inputs()));
    const send = (projectPath: string) =>
      act(async () => {
        await new QueryMutationObserver(queryClient, {
          mutationKey: [MESSAGE_LEAD_KEY],
          mutationFn: async () => null,
        }).mutate({ projectPath, text: "hi" } as never);
      });
    await send("/other");
    expect(result.current.done).toBe(false);
    await send("/p");
    expect(result.current.done).toBe(true);
    expect(typeof result.current.sentAt).toBe("number");
  });

  it("leadPanelAdjusted fires when the panel is hidden or resized, and stays done when it comes back", () => {
    const { result } = renderHook(() => useSignal("leadPanelAdjusted", inputs()));
    expect(result.current.done).toBe(false);
    act(() => useUiStore.getState().toggleLeadPanel());
    expect(result.current.done).toBe(true);
    act(() => useUiStore.getState().toggleLeadPanel());
    expect(result.current.done).toBe(true);
  });

  it("leadOpened fires on a click on this project's row in the sidebar, not another's", () => {
    const row = (path: string) => {
      const a = document.createElement("a");
      a.dataset.tour = "sidebar.project";
      a.dataset.tourScope = path;
      a.innerHTML = "<span>name</span>";
      document.body.appendChild(a);
      return a;
    };
    const other = row("/other");
    const mine = row("/p");
    const { result } = renderHook(() => useSignal("leadOpened", inputs({ onBoard: true })));
    act(() => other.querySelector("span")!.click());
    expect(result.current.done).toBe(false);
    act(() => mine.querySelector("span")!.click());
    expect(result.current.done).toBe(true);
    document.body.innerHTML = "";
  });

  it("leadExpanded fires when a collapsed dock is expanded", () => {
    const { result, rerender } = renderHook((p: SignalInputs) => useSignal("leadExpanded", p), {
      initialProps: inputs({ leadExpanded: true }),
    });
    expect(result.current.done).toBe(false);
    rerender(inputs({ leadExpanded: false }));
    rerender(inputs({ leadExpanded: true }));
    expect(result.current.done).toBe(true);
  });

  it("leadAnswered fires once a question the Lead asked during the step is answered", () => {
    const { result, rerender } = renderHook((p: SignalInputs) => useSignal("leadAnswered", p), {
      initialProps: inputs(),
    });
    rerender(inputs({ leadAsking: true }));
    expect(result.current.done).toBe(false);
    rerender(inputs({ leadAsking: false }));
    expect(result.current.done).toBe(true);
  });
});
