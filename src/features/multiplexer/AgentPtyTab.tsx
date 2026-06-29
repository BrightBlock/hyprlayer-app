import { useEffect, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { RestartIcon, ShieldIcon } from "../../components/icons";
import { commands, type TerminalEvent } from "../../lib/bindings";
import { unwrap } from "../../lib/ipc";
import { b64ToBytes, strToB64 } from "../session/terminalCodec";
import { useTerminalThemeSync, useTerminalZoom } from "../session/terminalChrome";
import { createTerminalEngine, type TerminalEngine } from "../session/terminalEngine";
import { xtermThemeFromCss } from "../session/terminalTheme";
import { useScriptsStore } from "../session/scriptsStore";
import { closeAgent, ensureAgent, setAgentBypass } from "./agentRuntime";
import { AgentStatePill, EngineGlyph } from "./agentStateUi";
import { engineActions, relaunchArgs, type ResolvedEngine } from "./engines";
import { useAgentState } from "./multiplexerStore";

/**
 * Renders a Multiplexer agent: an xterm view of the engine's PTY plus a slim
 * status bar (engine · state · Restart · bypass). The PTY + state detection are
 * owned by `agentRuntime` (so they outlive this component when the tab goes
 * background); here we open a render-only Channel for xterm while on screen, in
 * the same attach-or-spawn shape as `TerminalTab`. The branch is omitted — the
 * native CLIs already display it.
 */
export function AgentPtyTab({
  workstreamId,
  shellId,
  engine,
  projectPath,
  workstreamTitle,
  terminalId,
  bypass,
  autoFocus,
  onTerminalId,
  onBypassChange,
  notifyOnBlocked,
}: {
  workstreamId: string;
  shellId: number;
  engine: ResolvedEngine;
  projectPath: string;
  workstreamTitle: string;
  terminalId?: string;
  /** Persisted bypass-permissions state, restored on cold start so the relaunch
   *  resumes in bypass mode (the red shield). */
  bypass?: boolean;
  autoFocus?: boolean;
  onTerminalId: (id: string) => void;
  /** Persist a change to the agent's bypass mode (Restart / shield), so it
   *  survives a full quit. */
  onBypassChange?: (bypass: boolean) => void;
  notifyOnBlocked: boolean;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  const hostRef = useRef<HTMLDivElement>(null);
  const engineRef = useRef<TerminalEngine | null>(null);
  const idRef = useRef<string | undefined>(terminalId);
  const appliedFontSizeRef = useRef<number>(0);
  // Set by the main effect (which owns the engine + channel); invoked by the
  // status-bar Restart / bypass buttons, which live outside that effect.
  const restartRef = useRef<((extraArgs: string[], bypass: boolean) => void) | null>(null);

  // Zoom keybindings + apply, and live theme recolor — shared with TerminalTab.
  const fontSize = useTerminalZoom(engineRef, idRef, appliedFontSizeRef);
  useTerminalThemeSync(engineRef);

  const agent = useAgentState(workstreamId, shellId);
  const state = agent?.state ?? "idle";
  const bypassOn = !!agent?.bypass;
  const actions = engineActions(engine.id);
  // The scripts dock's open flag. Showing/hiding it resizes the pane instantly,
  // which can land between ResizeObserver ticks and leave the PTY at the old row
  // count (the CLI's composer then strands mid-screen) — so we refit explicitly
  // when this flips (effect below).
  const dockOpen = useScriptsStore((s) => !!s.dock[workstreamId]?.open);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const eng = createTerminalEngine({ fontSize, theme: xtermThemeFromCss() });
    eng.mount(host);
    engineRef.current = eng;
    appliedFontSizeRef.current = fontSize;
    let lastGrid = eng.fit();
    if (autoFocus) eng.focus();

    let disposed = false;
    // Render-only channel: writes PTY bytes to xterm. Detection runs separately
    // in agentRuntime, so this tab going background never stops state updates.
    // One Tauri Channel backs exactly one invoke (see TerminalTab), so mint a
    // fresh one for every attach. Guard against output that lands during the
    // unmount frame — it would otherwise write into a disposed terminal.
    const newChannel = (): Channel<TerminalEvent> => {
      const channel = new Channel<TerminalEvent>();
      channel.onmessage = (ev) => {
        if (disposed) return;
        if (ev.kind === "output") eng.write(b64ToBytes(ev.data));
        else if (ev.kind === "exited") eng.write("\r\n\x1b[2m[agent exited]\x1b[0m\r\n");
      };
      return channel;
    };
    void (async () => {
      // Wait two frames so xterm has computed its real grid before we spawn the
      // PTY at that size (see TerminalTab for the garbage-dims rationale).
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      if (disposed) return;
      lastGrid = eng.fit();

      // A persisted id means there was a prior session: after a full quit the PTY
      // is dead, so ensureAgent relaunches and we resume (--continue) + re-enter
      // bypass mode if it was on. On a webview reload it reattaches to the live
      // PTY instead and these args are unused.
      const id = await ensureAgent({
        workstreamId,
        shellId,
        engine: { ...engine, args: relaunchArgs(engine, { resume: !!idRef.current, bypass: !!bypass }) },
        projectPath,
        workstreamTitle,
        cols: lastGrid.cols,
        rows: lastGrid.rows,
        persistedTerminalId: idRef.current,
        notifyOnBlocked,
        bypass: !!bypass,
      });
      if (disposed || !id) return;
      idRef.current = id;
      onTerminalId(id);

      // Attach a render subscriber: replays scrollback then tails live output.
      await unwrap(commands.terminalAttach(id, newChannel())).catch(() => false);
      if (disposed) return;
      lastGrid = eng.fit();
      // Nudge on (re)attach — the child's last viewer may have been a different
      // grid, and a single SIGWINCH may not redraw it.
      void commands.agentResize(id, lastGrid.cols, lastGrid.rows);
    })();

    // Restart the agent, optionally with extra args (resume / bypass-permissions):
    // kill the old PTY, relaunch the engine fresh in the worktree, wipe the
    // screen, and re-attach the render channel. The persisted ShellTab keeps the
    // engine's base args, so the bypass flags apply to this run only — a plain
    // remount/relaunch goes back to normal permissions.
    restartRef.current = (extraArgs: string[], bypass: boolean) => {
      if (disposed) return;
      void (async () => {
        closeAgent(workstreamId, shellId);
        idRef.current = undefined;
        eng.reset();
        const relaunch =
          extraArgs.length > 0 ? { ...engine, args: [...engine.args, ...extraArgs] } : engine;
        const newId = await ensureAgent({
          workstreamId,
          shellId,
          engine: relaunch,
          projectPath,
          workstreamTitle,
          cols: lastGrid.cols,
          rows: lastGrid.rows,
          notifyOnBlocked,
          bypass,
        });
        if (disposed || !newId) return;
        idRef.current = newId;
        onTerminalId(newId);
        setAgentBypass(workstreamId, shellId, bypass);
        // Persist the new bypass state so a full quit relaunches in the same mode.
        onBypassChange?.(bypass);
        await unwrap(commands.terminalAttach(newId, newChannel())).catch(() => false);
        if (disposed) return;
        lastGrid = eng.fit();
        void commands.agentResize(newId, lastGrid.cols, lastGrid.rows);
      })();
    };

    const unsubInput = eng.onInput((data) => {
      if (idRef.current) void commands.terminalWrite(idRef.current, strToB64(data));
    });

    // Refit on container resize, coalesced to a frame; only emit a PTY resize
    // when the grid actually changed (host is absolutely positioned so fitting
    // can't feed back into layout — see TerminalTab).
    let raf = 0;
    const doFit = () => {
      raf = 0;
      if (disposed || host.clientWidth === 0 || host.clientHeight === 0) return;
      const next = eng.fit();
      if (next.cols !== lastGrid.cols || next.rows !== lastGrid.rows) {
        lastGrid = next;
        if (idRef.current) void commands.terminalResize(idRef.current, next.cols, next.rows);
      }
      // The WebGL renderer (addon ≥0.19) re-rasterises its glyph atlas across
      // resizes on its own — just repaint the newly-exposed rows.
      eng.refresh();
    };
    const scheduleFit = () => {
      if (!raf) raf = requestAnimationFrame(doFit);
    };
    const ro = new ResizeObserver(scheduleFit);
    // Observe the normal-flow root, not the absolutely-positioned host: WebKit
    // doesn't reliably fire ResizeObserver for an absolute element resized only
    // because a sibling (the scripts dock) was added/removed. Falls back to host.
    ro.observe(rootRef.current ?? host);

    // Repaint on focus-regain (WKWebView throttles rAF while unfocused).
    let unlistenFocus: (() => void) | undefined;
    let focusCancelled = false;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused || disposed) return;
        // Refit (its repaint handles a DPR change from a monitor move) + refresh
        // so a frame throttled while unfocused doesn't leave the terminal stale.
        scheduleFit();
        eng.refresh();
      })
      .then((fn) => {
        if (focusCancelled) fn();
        else unlistenFocus = fn;
      });

    return () => {
      // Unmount drops the render channel only — the PTY + detection live on in
      // agentRuntime (closing the tab is what kills it, via closeAgent).
      disposed = true;
      focusCancelled = true;
      restartRef.current = null;
      unlistenFocus?.();
      if (raf) cancelAnimationFrame(raf);
      unsubInput();
      ro.disconnect();
      eng.dispose();
      engineRef.current = null;
    };
    // Re-mount only on a real identity change. terminalId is read once via the
    // ref; engine/notify settings are forwarded to ensureAgent without remount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workstreamId, shellId]);

  // Refit the PTY whenever the scripts dock shows/hides. The pane resizes
  // instantly (no animation), which the ResizeObserver can miss, leaving the PTY
  // at the old row count — so the CLI keeps drawing its composer where the dock
  // used to start. Re-fit + resize + repaint once the new layout has settled.
  useEffect(() => {
    const eng = engineRef.current;
    if (!eng) return;
    let cancelled = false;
    const raf = requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        if (cancelled) return;
        const g = eng.fit();
        // Nudge-resize (off-by-one then back) so Claude/Codex actually repaint at
        // the new size — a plain resize leaves the composer stranded mid-pane.
        if (idRef.current) void commands.agentResize(idRef.current, g.cols, g.rows);
        // A dock toggle never changes cell metrics, so repaint without clearing
        // the atlas — clearing here is what made the terminals flash.
        eng.refresh();
      }),
    );
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
    };
  }, [dockOpen]);

  return (
    <div ref={rootRef} className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface">
      <div className="relative min-h-0 flex-1">
        <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
      </div>
      {/* Status bar: engine · state · Restart · bypass. Branch is omitted — the
          native CLI already shows it; model/ctx aren't reliably parseable. */}
      <div className="flex h-7 shrink-0 items-center gap-3 border-t border-edge px-3 font-mono text-[11px] text-ink-dim">
        <span className="inline-flex items-center gap-1.5">
          <EngineGlyph glyph={engine.glyph} accentToken={engine.accentToken} />
          <span style={{ color: `var(${engine.accentToken})` }}>{engine.label}</span>
        </span>
        <AgentStatePill state={state} />
        <div className="ml-auto flex items-center gap-1.5 font-sans">
          <button
            type="button"
            onClick={() => restartRef.current?.(actions.resumeArgs, false)}
            title="Restart the agent (resumes the session)"
            className="inline-flex items-center gap-1.5 rounded-md border border-edge px-2 py-0.5 hover:bg-edge/60 hover:text-ink"
          >
            <RestartIcon size={11} />
            Restart
          </button>
          {actions.bypassArgs.length > 0 && (
            <button
              type="button"
              onClick={() =>
                restartRef.current?.([...actions.resumeArgs, ...actions.bypassArgs], true)
              }
              aria-pressed={bypassOn}
              title={
                bypassOn
                  ? "Bypass-permissions mode is ON (auto-approving everything) — restart again to refresh; Restart to exit"
                  : "Restart in bypass-permissions mode (YOLO) — auto-approves everything"
              }
              aria-label="Restart in bypass-permissions mode"
              className={`inline-flex items-center justify-center rounded-md border border-bad/40 px-2 py-0.5 text-bad ${
                bypassOn ? "bg-bad/15" : "hover:bg-bad/10"
              }`}
            >
              <ShieldIcon size={12} filled={bypassOn} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
