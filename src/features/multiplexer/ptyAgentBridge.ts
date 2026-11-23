import { adoptPtyTitle } from "@/features/session/tabTitles";
import {
  openAgentTerminalTab,
  setPtyTabSessionId,
  setPtyTabStarted,
} from "@/features/session/workstreamTabs";
import { commands, events } from "@/lib/bindings";
import type { PtyAgent } from "@/lib/domain/agentControl";
import type { EventBridge } from "@/lib/events";
import { unwrap } from "@/lib/ipc";
import { queryClient } from "@/lib/queryClient";
import { maybeNotifyBlocked, onPtyAgentChanged } from "./agentRuntime";
import { applyPtyAgent, applyPtyAgentSnapshot, type PtyAgentApplied } from "./multiplexerStore";

function notifyOnBlock(applied: PtyAgentApplied | undefined): void {
  if (applied && applied.to === "blocked" && applied.from !== "blocked") {
    void maybeNotifyBlocked(applied.key);
  }
}

// Straight to the tab: an unshown background tab has no runtime to carry the id until a quit.
function persistTab(a: PtyAgent): void {
  if (a.providerSessionId) setPtyTabSessionId(a.workstreamId, a.terminalId, a.providerSessionId);
  if (a.title) adoptPtyTitle(a.workstreamId, a.terminalId, a.title);
  if (a.hooked !== false && a.state === "working") setPtyTabStarted(a.workstreamId, a.terminalId);
}

// Keeps Agent sessions' list current without a refetch per change.
function patchListed(a: PtyAgent): void {
  for (const scope of [a.workstreamId, null]) {
    queryClient.setQueryData<PtyAgent[]>(["pty-agents", scope], (rows) => {
      if (!rows) return rows;
      if (a.state === "exited") return rows.filter((r) => r.terminalId !== a.terminalId);
      if (!rows.some((r) => r.terminalId === a.terminalId)) return [...rows, a];
      return rows.map((r) => (r.terminalId === a.terminalId ? { ...r, ...a } : r));
    });
  }
}

// A desktop that missed an agent's OpenTerminalTab adopts the PTY now, unless it closed that tab before.
async function adoptSpawned(list: PtyAgent[]): Promise<void> {
  const spawned = list.filter((a) => a.spawnedBy);
  if (spawned.length === 0) return;
  const known = new Set((await unwrap(commands.listAllWorkstreams())).map((w) => w.item.id));
  for (const a of spawned) {
    if (!known.has(a.workstreamId)) continue;
    // No event carries the launch's bypass here; off never relaunches it into bypass.
    openAgentTerminalTab(a.workstreamId, {
      terminalId: a.terminalId,
      engineId: a.engineId,
      sessionId: a.providerSessionId ?? undefined,
      bypass: false,
    });
  }
}

/** A pick from Agent sessions: reattaches focused, with the listed state parked for the new tab to bind. */
export function reattachPtyAgent(a: PtyAgent): void {
  // As reconcile adopts one: no event carries the launch's bypass, and off never relaunches into it.
  openAgentTerminalTab(
    a.workstreamId,
    { terminalId: a.terminalId, engineId: a.engineId, sessionId: a.providerSessionId ?? undefined, bypass: false },
    { focus: true },
  );
  persistTab(a);
  if (a.hooked !== false) applyPtyAgent(a);
}

/** Startup and reconnect: no change replays, and a restarted server forgot every PTY. */
export async function reconcilePtyAgents(): Promise<void> {
  const list = await unwrap(commands.listPtyAgents(null));
  // An unhooked entry is listed only to be adopted; its state was never folded.
  const hooked = list.filter((a) => a.hooked !== false);
  for (const applied of applyPtyAgentSnapshot(hooked)) notifyOnBlock(applied);
  void queryClient.invalidateQueries({ queryKey: ["pty-agents"] });
  await adoptSpawned(list);
  // After adoption: a title has no tab to land in until adoptSpawned makes one.
  list.forEach(persistTab);
}

/** Every desktop hears the change; only the one whose tab binds the terminal notifies (#320). */
export const ptyAgentBridge: EventBridge = () => [
  events.ptyAgentChanged.listen(({ payload }) => {
    persistTab(payload);
    patchListed(payload);
    notifyOnBlock(applyPtyAgent(payload));
    onPtyAgentChanged(payload);
  }),
];
