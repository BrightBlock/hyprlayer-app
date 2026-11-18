import type { Bootstrap } from "@/lib/bindings";
import type { StageKey } from "@/lib/domain/stages";
import { findWorkstreamForNotify } from "@/lib/notifications";
import { queryClient } from "@/lib/queryClient";
import { useToast } from "@/lib/toast";
import {
  appendShell,
  defaultTabState,
  focusShell,
  focusedPaneId,
  nextShellId,
  removeShellState,
  updateWorkstreamTabs,
  useWorkstreamTabsStore,
  type ShellTab,
} from "@/features/session/workstreamTabs";
import { closeAgent, ensureAgent } from "./agentRuntime";
import { defaultEngine, resolveMultiplexer } from "./engines";
import { agentKey, useMultiplexerStore } from "./multiplexerStore";

export type ActionAgent = { stage: StageKey; input?: string; attachedDocs?: string[]; title: string };

/** The server waits this long for a TUI to take its first prompt. */
const PROMPT_WINDOW_MS = 60_000;
const launchedAt = new Map<string, number>();

// Prompts and reviews are one-offs; a stage action has one run at a time, as its native tab does.
function busy(workstreamId: string, shell: ShellTab): boolean {
  const key = agentKey(workstreamId, shell.id);
  const state = useMultiplexerStore.getState().agents[key]?.state;
  if (state === "working" || state === "blocked") return true;
  // Unhooked engines never report working, so past the paste window an idle tab is free.
  return !shell.ptyAgent?.started && Date.now() - (launchedAt.get(key) ?? 0) < PROMPT_WINDOW_MS;
}

/** Runs an app action in a new default-engine tab, resolving to whether it launched; null leaves it to a native chat. */
export function launchActionInCli(workstreamId: string, action: ActionAgent): Promise<boolean> | null {
  const cfg = resolveMultiplexer(queryClient.getQueryData<Bootstrap>(["bootstrap"])?.settings);
  if (!cfg.enabled || !cfg.actionsInCli) return null;
  const engine = defaultEngine(cfg);
  const ws = findWorkstreamForNotify(workstreamId);
  if (!engine || !ws) return null;
  const keyed = action.stage !== "general";
  const running = keyed
    ? useWorkstreamTabsStore
        .getState()
        .byWs[workstreamId]?.shells.find((s) => s.ptyAgent?.action === action.stage && busy(workstreamId, s))
    : undefined;
  if (running) {
    updateWorkstreamTabs((all) => {
      const current = all[workstreamId];
      return current ? { ...all, [workstreamId]: focusShell(current, running.id) } : all;
    });
    return Promise.resolve(true);
  }
  let shellId = 0;
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    shellId = nextShellId(current.shells, current.archived);
    const shell: ShellTab = {
      id: shellId,
      title: action.title,
      titleLocked: true,
      mode: "general",
      draft: "",
      docs: [],
      attachments: [],
      paneId: focusedPaneId(current),
      // Not `started`: a relaunch must start fresh, not `--continue` another session.
      ptyAgent: { engineId: engine.id, started: false, ...(keyed ? { action: action.stage } : {}) },
    };
    return { ...all, [workstreamId]: appendShell(current, shell) };
  });
  launchedAt.set(agentKey(workstreamId, shellId), Date.now());
  // ensureAgent registers the runtime before it awaits, so the tab's mount and restore join this launch.
  // A reload before the terminal binds orphans it, as it would any launch in flight.
  return ensureAgent({
    workstreamId,
    shellId,
    engine,
    projectPath: ws.projectPath,
    workstreamTitle: ws.title,
    notifyOnBlocked: cfg.notifyOnBlocked,
    session: {
      resume: false,
      bypass: false,
      firstPrompt: {
        stage: action.stage,
        input: action.input ?? "",
        attachedDocs: action.attachedDocs ?? [],
      },
    },
    started: false,
  }).then((launched) => {
    if (launched) return true;
    // Left open, the tab would relaunch a bare CLI without the prompt.
    closeAgent(workstreamId, shellId);
    updateWorkstreamTabs((all) => {
      const current = all[workstreamId];
      return current ? { ...all, [workstreamId]: removeShellState(current, shellId) } : all;
    });
    useToast
      .getState()
      .show(`${action.title} didn't start`, `${engine.label} failed to launch. Check that it is installed.`, {
        tone: "bad",
      });
    return false;
  });
}
