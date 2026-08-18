import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { RestartIcon, ShieldIcon } from "@/components/icons";
import { Spinner } from "@/components/ui";
import { commands, type TerminalEvent } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { b64ToBytes, strToB64 } from "@/lib/terminal/terminalCodec";
import {
  attachTerminalClipboard,
  useTerminalThemeSync,
  useTerminalZoom,
} from "@/lib/terminal/terminalChrome";
import { createTerminalEngine, type TerminalEngine } from "@/lib/terminal/terminalEngine";
import { xtermThemeFromCss } from "@/lib/terminal/terminalTheme";
import { useScriptsStore } from "@/features/session/scriptsStore";
import { closeAgent, ensureAgent, resolveResume, setAgentBypass } from "./agentRuntime";
import { AgentStatePill, EngineGlyph } from "./agentStateUi";
import {
  engineBypassArgs,
  engineMintsViaExec,
  engineResumesById,
  relaunchArgs,
  type ResolvedEngine,
} from "./engines";
import { markAgentStarted, useAgentState } from "./multiplexerStore";

// Reading back a capture-mode engine's auto-assigned id: its on-disk record
// appears on the first message, so poll a handful of times after the conversation
// begins (≈30s window) before giving up and falling back to most-recent resume.
const CAPTURE_INTERVAL_MS = 1500;
const CAPTURE_TRIES = 20;

/**
 * Renders a Multiplexer agent: an xterm view of the engine's PTY plus a slim
 * status bar (engine · state · Restart · bypass). The PTY + state detection are
 * owned by `agentRuntime` (so they outlive this component when the tab goes
 * background); here we open a render-only Channel for xterm while on screen, in
 * the same attach-or-spawn shape as `TerminalTab`. The branch is omitted, the
 * native CLIs already display it.
 */
export function AgentPtyTab({
  workstreamId,
  shellId,
  engine,
  projectPath,
  workstreamTitle,
  terminalId,
  sessionId,
  started: startedProp,
  bypass,
  autoFocus,
  excludeSessionIds,
  onTerminalId,
  onBypassChange,
  onStarted,
  onSessionId,
  notifyOnBlocked,
}: {
  workstreamId: string;
  shellId: number;
  engine: ResolvedEngine;
  projectPath: string;
  workstreamTitle: string;
  terminalId?: string;
  /** This tab's own conversation id, so it resumes ITS conversation (not the most
   *  recent in the cwd). Minted at tab creation for "assign" engines (Claude,
   *  Gemini); captured from disk after launch for "capture" engines. */
  sessionId?: string;
  /** Persisted "a conversation has begun" flag, so a cold-start relaunch resumes
   *  (by id) only when there's something to resume. */
  started?: boolean;
  /** Persisted bypass-permissions state, restored on cold start so the relaunch
   *  resumes in bypass mode (the red shield). */
  bypass?: boolean;
  autoFocus?: boolean;
  /** Conversation ids already owned by sibling tabs (live + archived), so a
   *  capture-mode engine doesn't claim one that's taken. */
  excludeSessionIds?: string[];
  onTerminalId: (id: string) => void;
  /** Persist a change to the agent's bypass mode (Restart / shield), so it
   *  survives a full quit. */
  onBypassChange?: (bypass: boolean) => void;
  /** Persist that this tab's conversation has begun (first keystroke), so a
   *  cold-start relaunch knows to resume it. */
  onStarted?: () => void;
  /** Persist a conversation id read back from disk (capture-mode engines). */
  onSessionId?: (id: string) => void;
  notifyOnBlocked: boolean;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  const hostRef = useRef<HTMLDivElement>(null);
  const engineRef = useRef<TerminalEngine | null>(null);
  const idRef = useRef<string | undefined>(terminalId);
  // The live conversation id, read by the relaunch builders. A capture-mode engine
  // (codex/antigravity) learns its id AFTER mount, so the restart closure must see
  // the latest value, not the one captured when the effect first ran.
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;
  // When the engine last launched fresh, the lower bound for capturing the id it
  // auto-assigns (capture-mode engines). Set on each fresh (re)launch.
  const launchedAtRef = useRef(0);
  // Sibling tabs' ids to skip when capturing, read live (changes as tabs come/go).
  const excludeRef = useRef<string[]>(excludeSessionIds ?? []);
  excludeRef.current = excludeSessionIds ?? [];
  const appliedFontSizeRef = useRef<number>(0);
  // Set by the main effect (which owns the engine + channel); invoked by the
  // status-bar Restart / bypass buttons, which live outside that effect.
  const restartRef = useRef<((opts: { resume: boolean; bypass: boolean }) => void) | null>(null);

  // Zoom keybindings + apply, and live theme recolor, shared with TerminalTab.
  const fontSize = useTerminalZoom(engineRef, idRef, appliedFontSizeRef);
  useTerminalThemeSync(engineRef);

  const agent = useAgentState(workstreamId, shellId);
  const state = agent?.state ?? "idle";
  const bypassOn = !!agent?.bypass;
  // Whether a conversation has begun, gates the Restart/shield resume so a fresh,
  // never-typed tab doesn't relaunch into a resume (which exits "No conversation
  // found"). Flips on the first keystroke (eng.onInput below).
  const started = !!agent?.started;
  const hasBypass = engineBypassArgs(engine.id).length > 0;
  // The cold-start resume signal: the persisted "started" flag, falling back to
  // "a PTY id was persisted" for tabs from before that flag existed.
  const startedSeed = startedProp ?? !!terminalId;
  // Fire the started persist + runtime mark exactly once per mount (idempotent
  // thereafter); seeded true when the tab already had a conversation.
  const markedStartedRef = useRef(startedSeed);
  // Shown while an exec-mint engine (Codex) seeds + names its conversation before
  // the interactive PTY launches, letting that probe finish keeps the session
  // from persisting "interrupted".
  const [minting, setMinting] = useState(false);
  // The scripts dock's open flag. Showing/hiding it resizes the pane instantly,
  // which can land between ResizeObserver ticks and leave the PTY at the old row
  // count (the CLI's composer then strands mid-screen), so we refit explicitly
  // when this flips (effect below).
  const dockOpen = useScriptsStore((s) => !!s.dock[workstreamId]?.open);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const eng = createTerminalEngine({
      fontSize,
      theme: xtermThemeFromCss(),
      // Codex leaves a visible cursor parked in its footer while mid-turn, which
      // blinks/flickers there; steady it for Codex (other CLIs hide theirs).
      cursorBlink: engine.id !== "codex",
    });
    eng.mount(host);
    // Ctrl+Shift+C / Ctrl+Shift+V off macOS, the emulator has no copy
    // path of its own there (see attachTerminalClipboard).
    attachTerminalClipboard(eng);
    engineRef.current = eng;
    appliedFontSizeRef.current = fontSize;
    let lastGrid = eng.fit();
    if (autoFocus) eng.focus();

    let disposed = false;
    // Render-only channel: writes PTY bytes to xterm. Detection runs separately
    // in agentRuntime, so this tab going background never stops state updates.
    // One Tauri Channel backs exactly one invoke (see TerminalTab), so mint a
    // fresh one for every attach. Guard against output that lands during the
    // unmount frame. It would otherwise write into a disposed terminal.
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
      launchedAtRef.current = Date.now();

      // Exec-mint engines (Codex) can't take a caller-set id, so before the first
      // launch we mint one deterministically via a `codex exec` probe and then
      // resume it, no disk-attribution race. Skipped once we already have an id
      // (restore / cold-start); a null result falls through to a bare launch and
      // the on-disk capture below as a backstop.
      if (!sessionIdRef.current && engineMintsViaExec(engine.id)) {
        setMinting(true);
        const minted = await unwrap(
          commands.mintCodexSession(workstreamId, engine.command),
        ).catch(() => null);
        setMinting(false);
        if (disposed) return;
        if (minted) {
          sessionIdRef.current = minted;
          onSessionId?.(minted);
        }
      }

      // After a full quit the PTY is dead, so ensureAgent relaunches: resume THIS
      // tab's conversation by its sessionId iff it's actually on disk (else launch
      // fresh) + re-enter bypass mode if it was on. On a webview reload it
      // reattaches to the live PTY instead and these args are unused.
      const resume = await resolveResume(engine.id, sessionIdRef.current, workstreamId, startedSeed);
      if (disposed) return;
      const id = await ensureAgent({
        workstreamId,
        shellId,
        engine: {
          ...engine,
          args: relaunchArgs(engine, { sessionId: sessionIdRef.current, resume, bypass: !!bypass }),
        },
        projectPath,
        workstreamTitle,
        cols: lastGrid.cols,
        rows: lastGrid.rows,
        persistedTerminalId: idRef.current,
        notifyOnBlocked,
        bypass: !!bypass,
        // A started conversation ⇒ the Restart/shield resume it by id; a fresh tab
        // waits for the first keystroke (markedStartedRef below).
        started: startedSeed,
      });
      if (disposed || !id) return;
      idRef.current = id;
      onTerminalId(id);

      // Attach a render subscriber: replays scrollback then tails live output.
      await unwrap(commands.terminalAttach(id, newChannel())).catch(() => false);
      if (disposed) return;
      lastGrid = eng.fit();
      // Nudge on (re)attach, the child's last viewer may have been a different
      // grid, and a single SIGWINCH may not redraw it.
      void commands.agentResize(id, lastGrid.cols, lastGrid.rows);
    })();

    // Restart the agent in the worktree: kill the old PTY, relaunch the engine
    // (resume continues the prior conversation; bypass adds the skip-permission
    // flags), wipe the screen, and re-attach the render channel. The persisted
    // ShellTab keeps the engine's base args, so resume/bypass apply to this run
    // only, a plain remount/relaunch goes back to a fresh, normal-permission run.
    restartRef.current = (opts: { resume: boolean; bypass: boolean }) => {
      if (disposed) return;
      void (async () => {
        closeAgent(workstreamId, shellId);
        idRef.current = undefined;
        eng.reset();
        launchedAtRef.current = Date.now();
        const newId = await ensureAgent({
          workstreamId,
          shellId,
          engine: { ...engine, args: relaunchArgs(engine, { sessionId: sessionIdRef.current, ...opts }) },
          projectPath,
          workstreamTitle,
          cols: lastGrid.cols,
          rows: lastGrid.rows,
          notifyOnBlocked,
          bypass: opts.bypass,
          // Resuming carries the prior conversation into the new run; a fresh
          // (non-resume) relaunch begins empty and waits for the next keystroke.
          started: opts.resume,
        });
        if (disposed || !newId) return;
        idRef.current = newId;
        onTerminalId(newId);
        setAgentBypass(workstreamId, shellId, opts.bypass);
        // Persist the new bypass state so a full quit relaunches in the same mode.
        onBypassChange?.(opts.bypass);
        await unwrap(commands.terminalAttach(newId, newChannel())).catch(() => false);
        if (disposed) return;
        lastGrid = eng.fit();
        void commands.agentResize(newId, lastGrid.cols, lastGrid.rows);
      })();
    };

    // All input (keystrokes, paste, AND xterm's auto-replies to the CLI's startup
    // capability queries) goes to the PTY verbatim. Auto-replies fire with no user
    // action, so this is NOT where we decide a conversation has begun.
    const unsubInput = eng.onInput((data) => {
      if (idRef.current) void commands.terminalWrite(idRef.current, strToB64(data));
    });

    // A real key press (not an auto-reply) is the honest "the user is composing"
    // signal: the first typed character or Enter marks the conversation as begun,
    // so Restart/shield/cold-start resume it. Without this, a fresh agent's startup
    // auto-replies would flip "started" and we'd resume a session that never
    // existed ("No conversation found with session ID …").
    const unsubKey = eng.onKey(({ domEvent }) => {
      if (markedStartedRef.current) return;
      const composing =
        !domEvent.ctrlKey &&
        !domEvent.metaKey &&
        !domEvent.altKey &&
        (domEvent.key.length === 1 || domEvent.key === "Enter");
      if (!composing) return;
      markedStartedRef.current = true;
      markAgentStarted(workstreamId, shellId);
      onStarted?.();
    });

    // Refit on container resize, coalesced to a frame; only emit a PTY resize
    // when the grid actually changed (host is absolutely positioned so fitting
    // can't feed back into layout, see TerminalTab).
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
      // resizes on its own, just repaint the newly-exposed rows.
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
      // Unmount drops the render channel only, the PTY + detection live on in
      // agentRuntime (closing the tab is what kills it, via closeAgent).
      disposed = true;
      focusCancelled = true;
      restartRef.current = null;
      unlistenFocus?.();
      if (raf) cancelAnimationFrame(raf);
      unsubInput();
      unsubKey();
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
  // at the old row count, so the CLI keeps drawing its composer where the dock
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
        // the new size, a plain resize leaves the composer stranded mid-pane.
        if (idRef.current) void commands.agentResize(idRef.current, g.cols, g.rows);
        // A dock toggle never changes cell metrics, so repaint without clearing
        // the atlas, clearing here is what made the terminals flash.
        eng.refresh();
      }),
    );
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
    };
  }, [dockOpen]);

  // Fallback id capture from disk: for an engine we can resume by id that still
  // lacks one once its conversation has begun, Antigravity (its normal path) and
  // Codex if the exec-mint probe failed. Reads the id from the engine's on-disk
  // store and persists it so Restart / cold-start / unarchive resume THIS tab's
  // thread. Assign engines already hold an id, so the `!sessionId` guard skips
  // them. Polls briefly, skipping ids owned by sibling tabs.
  useEffect(() => {
    if (sessionId || !started || !engineResumesById(engine.id)) return;
    let cancelled = false;
    const afterMs = launchedAtRef.current || Date.now();
    void (async () => {
      for (let i = 0; i < CAPTURE_TRIES && !cancelled; i++) {
        await new Promise((r) => setTimeout(r, CAPTURE_INTERVAL_MS));
        if (cancelled) return;
        const id = await unwrap(
          commands.captureAgentSession(workstreamId, engine.id, afterMs, excludeRef.current),
        ).catch(() => null);
        if (cancelled) return;
        if (id) {
          sessionIdRef.current = id;
          onSessionId?.(id);
          return;
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // onSessionId is read live (its identity changes per render); the rest gate it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine.id, sessionId, started, workstreamId]);

  return (
    <div ref={rootRef} className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface">
      <div className="relative min-h-0 flex-1">
        <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
        {minting && (
          <div className="absolute inset-0 flex items-center justify-center bg-surface/70 backdrop-blur-[1px]">
            <span className="inline-flex items-center gap-2 font-mono text-xs text-ink-dim">
              <Spinner size={13} />
              Creating session…
            </span>
          </div>
        )}
      </div>
      {/* Status bar: engine · state · Restart · bypass. Branch is omitted, the
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
            onClick={async () => {
              const resume = await resolveResume(engine.id, sessionIdRef.current, workstreamId, started);
              // Keep the current bypass mode across a restart (the shield toggles it).
              restartRef.current?.({ resume, bypass: bypassOn });
            }}
            title={
              bypassOn
                ? "Restart the agent (resumes its conversation, stays in bypass mode)"
                : "Restart the agent (resumes its conversation if one exists)"
            }
            className="inline-flex items-center gap-1.5 rounded-md border border-edge px-2 py-0.5 hover:bg-edge/60 hover:text-ink"
          >
            <RestartIcon size={11} />
            Restart
          </button>
          {hasBypass && (
            <button
              type="button"
              onClick={async () => {
                const resume = await resolveResume(engine.id, sessionIdRef.current, workstreamId, started);
                // Toggle: flip bypass on/off (and relaunch). Restart keeps the mode.
                restartRef.current?.({ resume, bypass: !bypassOn });
              }}
              aria-pressed={bypassOn}
              title={
                bypassOn
                  ? "Bypass-permissions mode is ON (auto-approving everything). Click to turn off"
                  : "Restart in bypass-permissions mode. Auto-approves every command"
              }
              aria-label="Toggle bypass-permissions mode"
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
