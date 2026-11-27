// Binds the perf probe to this app's stores and DOM. Imported only behind VITE_PERF_PROBE.
import { goToWorkstream } from "@/lib/appNav";
import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { startPerfProbe, type PerfHost } from "@/lib/perfProbe";
import { writeTerminal } from "@/lib/terminal/terminalInput";
import { sendMessage, useSessionStore } from "@/features/session/sessionStore";
import {
  appendShell,
  nextShellId,
  openSessionTab,
  splitPaneState,
  updateWorkstreamTabs,
} from "@/features/session/workstreamTabs";

const SCROLLER = '[data-pane="p1"] [data-pane-content] .overflow-y-auto.overscroll-contain';
const ROWS = "[data-item-id], [data-tooluse-id]";
const frame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));

async function until(ok: () => boolean, ms: number, what: string): Promise<void> {
  const end = Date.now() + ms;
  while (!ok()) {
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

let target: { projectPath: string; workstreamId: string; sessionId: string } | null = null;

async function locate() {
  if (target) return target;
  const views = await unwrap(commands.listAllWorkstreams());
  const ws = views[0]?.item;
  if (!ws) throw new Error("the fixture has no workstream");
  const sessions = await unwrap(commands.listSessions(ws.id));
  const session = sessions[0];
  if (!session) throw new Error("the fixture has no session");
  target = { projectPath: ws.projectPath, workstreamId: ws.id, sessionId: session.id };
  return target;
}

const host: PerfHost = {
  async projectsRendered() {
    // Rows in the DOM, then one frame so the mark follows the paint that shows them.
    await until(() => document.querySelector("[data-project-path]") !== null, 120_000, "the sidebar projects");
    await frame();
    await frame();
  },
  async openTranscript() {
    const t = await locate();
    goToWorkstream({ projectPath: t.projectPath, workstreamId: t.workstreamId });
    const sessions = await unwrap(commands.listSessions(t.workstreamId));
    const session = sessions.find((s) => s.id === t.sessionId);
    if (!session) throw new Error("the session vanished");
    await until(() => document.querySelector("[data-pane-content]") !== null, 30_000, "the workstream pane");
    openSessionTab(t.workstreamId, session);
    await until(() => (useSessionStore.getState().sessions[t.sessionId]?.items.length ?? 0) > 0, 60_000, "transcript items");
    await until(() => document.querySelectorAll(`${SCROLLER} ${ROWS}`).length > 0, 30_000, "transcript rows");
  },
  transcriptProof() {
    return {
      storeItems: target ? (useSessionStore.getState().sessions[target.sessionId]?.items.length ?? 0) : 0,
      domRows: document.querySelectorAll(`${SCROLLER} ${ROWS}`).length,
    };
  },
  scroller: () => document.querySelector<HTMLElement>(SCROLLER),
  async sendMessage(text) {
    const t = await locate();
    const outcome = await sendMessage(t.sessionId, text);
    if (outcome === "dropped") throw new Error("the session did not take the message");
  },
  transcriptShows: (text) => (document.querySelector(SCROLLER)?.textContent ?? "").includes(text),
  async openTerminal(id) {
    const t = await locate();
    updateWorkstreamTabs((all) => {
      const current = all[t.workstreamId];
      if (!current) return all;
      const shell = {
        id: nextShellId(current.shells, current.archived),
        title: "Terminal",
        mode: "general" as const,
        docs: [],
        attachments: [],
        terminalView: { terminalId: id },
        paneId: current.activePaneId,
      };
      // Its own pane below the transcript, so both stay mounted and visible.
      const added = appendShell(current, shell);
      return { ...all, [t.workstreamId]: splitPaneState(added, added.activePaneId, "col", "after", shell.id) };
    });
    await until(() => document.querySelector(".xterm") !== null, 30_000, "the terminal view");
    writeTerminal(id, "\r");
  },
  terminalGrid() {
    const measure = document.querySelector<HTMLElement>(".xterm-char-measure-element");
    const screen = document.querySelector<HTMLElement>(".xterm-screen");
    const row = document.querySelector<HTMLElement>(".xterm-rows > div");
    if (!measure || !screen || !row) return null;
    return {
      cols: Math.floor(screen.clientWidth / (measure.getBoundingClientRect().width / 32)),
      rows: Math.floor(screen.clientHeight / row.getBoundingClientRect().height),
    };
  },
};

export function startPerf(): void {
  void startPerfProbe(host).catch((e) => console.error("perf probe failed", e));
}
