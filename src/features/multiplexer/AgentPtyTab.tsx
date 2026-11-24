import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { TakeSizeButton } from "@/components/TakeSizeButton";
import { Spinner } from "@/components/ui";
import { monoFontStack } from "@/lib/appearance";
import { commands } from "@/lib/bindings";
import { onServerReconnected } from "@/lib/events";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { useToast } from "@/lib/toast";
import { TerminalFeed } from "@/lib/terminal/terminalFeed";
import { claimIfFocused, claimOnPress, writeTerminal } from "@/lib/terminal/terminalInput";
import {
  attachTerminalClipboard,
  useTerminalThemeSync,
  useTerminalZoom,
} from "@/lib/terminal/terminalChrome";
import { createTerminalEngine, type TerminalEngine } from "@/lib/terminal/terminalEngine";
import { xtermThemeFromCss } from "@/lib/terminal/terminalTheme";
import { useScriptsStore } from "@/features/session/scriptsStore";
import {
  closeAgent,
  ensureAgent,
  onAgentTerminalBack,
  onAgentTerminalGone,
  resolveResume,
  setAgentBypass,
  type AgentLaunchSession,
  type EnsureAgentOpts,
} from "./agentRuntime";
import { engineMintsViaExec, engineResumesById, type ResolvedEngine } from "./engines";
import { markAgentStarted, useAgentState } from "./multiplexerStore";
import { PtySessionControls, type RelaunchKeep } from "./PtySessionControls";

const CAPTURE_INTERVAL_MS = 1500;
const CAPTURE_TRIES = 20;
// A tab shown again paints its last screen while it reattaches, not an empty pane.
// Each parked one keeps a WebGL context, and WebKit drops the oldest past about 16.
const PARKED_MAX = 4;
const parked = new Map<string, { eng: TerminalEngine; next: number | undefined }>();

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
  adopted = false,
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
  sessionId?: string;
  started?: boolean;
  bypass?: boolean;
  adopted?: boolean;
  autoFocus?: boolean;
  excludeSessionIds?: string[];
  onTerminalId: (id: string) => void;
  onBypassChange?: (bypass: boolean) => void;
  onStarted?: () => void;
  onSessionId?: (id: string) => void;
  notifyOnBlocked: boolean;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  const hostRef = useRef<HTMLDivElement>(null);
  const engineRef = useRef<TerminalEngine | null>(null);
  const idRef = useRef<string | undefined>(terminalId);
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;
  const launchedAtRef = useRef(0);
  const excludeRef = useRef<string[]>(excludeSessionIds ?? []);
  excludeRef.current = excludeSessionIds ?? [];
  const restartRef = useRef<((opts: Omit<AgentLaunchSession, "sessionId">) => void) | null>(null);

  const fontSize = useTerminalZoom(engineRef, idRef);
  useTerminalThemeSync(engineRef);

  const agent = useAgentState(workstreamId, shellId);
  const bypassOn = !!agent?.bypass;
  const started = !!agent?.started;
  const hooked = !!agent?.hooked;
  const reportedSessionId = agent?.sessionId;
  const startedSeed = startedProp ?? !!terminalId;
  const markedStartedRef = useRef(startedSeed);
  const [minting, setMinting] = useState(false);
  const [sizeOwner, setSizeOwner] = useState<boolean | null>(null);
  const dockOpen = useScriptsStore((s) => !!s.dock[workstreamId]?.open);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const parkedId = idRef.current;
    const spare = parkedId ? parked.get(parkedId) : undefined;
    if (parkedId) parked.delete(parkedId);
    const eng =
      spare?.eng ??
      createTerminalEngine({
        fontSize,
        fontFamily: monoFontStack(),
        theme: xtermThemeFromCss(),
        cursorBlink: engine.id !== "codex",
      });
    eng.mount(host);
    if (spare) {
      eng.setTheme(xtermThemeFromCss());
      eng.setFontSize(fontSize);
    }
    attachTerminalClipboard(eng);
    engineRef.current = eng;
    let lastGrid = eng.fit();
    if (autoFocus) eng.focus();

    let disposed = false;
    // A terminal can end both ways at once (its exit races a reconnect's check); it says so once.
    let exitShownFor: string | undefined;
    const showExited = (id: string) => {
      if (disposed || exitShownFor === id) return;
      exitShownFor = id;
      eng.write("\r\n\x1b[2m[agent exited]\x1b[0m\r\n");
    };
    // Only the current terminal's streams are live, so an exit is that terminal's.
    const feed = new TerminalFeed(
      eng,
      () => {
        if (idRef.current) showExited(idRef.current);
      },
      undefined,
      setSizeOwner,
    );
    feed.next = spare?.next;
    const unsubGone = onAgentTerminalGone(workstreamId, shellId, (id) => {
      if (id === idRef.current) showExited(id);
    });
    // A restarted server resumed it in a new process: its output starts over, so the screen does too.
    const unsubBack = onAgentTerminalBack(workstreamId, shellId, (id) => {
      if (disposed || id !== idRef.current) return;
      exitShownFor = undefined;
      eng.reset();
      feed.restart();
      void unwrap(commands.terminalAttach(id, null, feed.channel()))
        .then((ok) => {
          if (!ok || disposed) return;
          lastGrid = eng.fit();
          void commands.agentResize(id, lastGrid.cols, lastGrid.rows);
        })
        .catch((e) => log.warn("agent", "reattaching a resumed agent failed", e));
    });
    // An unhooked codex launch without a session waits on the server's `codex exec` mint (up to 90 s).
    const launch = async (opts: EnsureAgentOpts) => {
      const { sessionId, resume } = opts.session;
      const mints =
        !opts.persistedTerminalId && !sessionId && !resume && engineMintsViaExec(engine.id);
      if (mints) setMinting(true);
      try {
        return await ensureAgent(opts);
      } finally {
        if (mints && !disposed) setMinting(false);
      }
    };
    let startFailed = false;
    const start = async () => {
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      if (disposed) return;
      lastGrid = eng.fit();
      launchedAtRef.current = Date.now();

      // A codex with no id starts on a fresh mint; `resume --last` could pick up another tab's session.
      const resume =
        !sessionIdRef.current && engineMintsViaExec(engine.id)
          ? false
          : await resolveResume(engine.id, sessionIdRef.current, workstreamId, startedSeed);
      if (disposed) return;
      const launched = await launch({
        workstreamId,
        shellId,
        engine,
        projectPath,
        workstreamTitle,
        cols: lastGrid.cols,
        rows: lastGrid.rows,
        persistedTerminalId: idRef.current,
        notifyOnBlocked,
        session: { sessionId: sessionIdRef.current, resume, bypass: !!bypass },
        started: startedSeed,
        adopted,
      });
      if (disposed) return;
      if (!launched) {
        startFailed = true;
        eng.write("\r\n\x1b[31mThe agent didn't start. It tries again when the server reconnects.\x1b[0m\r\n");
        return;
      }
      if (launched.exited) {
        showExited(launched.terminalId);
        return;
      }
      const id = launched.terminalId;
      idRef.current = id;
      onTerminalId(id);
      eng.bind(id);

      const { from, channel } =
        spare && id === parkedId ? feed.resume() : { from: null, channel: feed.channel() };
      await unwrap(commands.terminalAttach(id, from, channel)).catch(() => false);
      if (disposed) return;
      lastGrid = eng.fit();
      void commands.agentResize(id, lastGrid.cols, lastGrid.rows);
      claimIfFocused(id);
    };
    void start();

    // The stream died with the old connection, or never opened; a gone agent is reported through `onAgentTerminalGone`.
    const offReconnect = onServerReconnected(() => {
      if (disposed) return;
      if (startFailed) {
        startFailed = false;
        void start();
        return;
      }
      const id = idRef.current;
      if (!id || exitShownFor === id) return;
      const { from, channel } = feed.resume();
      void unwrap(commands.terminalAttach(id, from, channel))
        .then((ok) => {
          if (!ok || disposed) return;
          // A new connection is a new viewer with no fit. No claim: a reconnect isn't the user acting.
          void commands.terminalResize(id, lastGrid.cols, lastGrid.rows);
        })
        .catch((e) => log.warn("agent", "reattaching after a reconnect failed", e));
    });

    restartRef.current = (opts: Omit<AgentLaunchSession, "sessionId">) => {
      if (disposed) return;
      void (async () => {
        closeAgent(workstreamId, shellId);
        idRef.current = undefined;
        eng.reset();
        feed.restart();
        launchedAtRef.current = Date.now();
        const relaunched = await launch({
          workstreamId,
          shellId,
          engine,
          projectPath,
          workstreamTitle,
          cols: lastGrid.cols,
          rows: lastGrid.rows,
          notifyOnBlocked,
          session: { sessionId: sessionIdRef.current, ...opts },
          started: opts.resume,
        });
        if (disposed || !relaunched) return;
        const newId = relaunched.terminalId;
        idRef.current = newId;
        onTerminalId(newId);
        eng.bind(newId);
        setAgentBypass(workstreamId, shellId, opts.bypass);
        onBypassChange?.(opts.bypass);
        await unwrap(commands.terminalAttach(newId, null, feed.channel())).catch(() => false);
        if (disposed) return;
        lastGrid = eng.fit();
        void commands.agentResize(newId, lastGrid.cols, lastGrid.rows);
        claimIfFocused(newId);
      })();
    };

    const offPress = claimOnPress(host, () => idRef.current);
    const unsubInput = eng.onInput((data) => {
      if (idRef.current) writeTerminal(idRef.current, data);
    });

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

    let raf = 0;
    const doFit = () => {
      raf = 0;
      if (disposed || host.clientWidth === 0 || host.clientHeight === 0) return;
      const next = eng.fit();
      if (next.cols !== lastGrid.cols || next.rows !== lastGrid.rows) {
        lastGrid = next;
        if (idRef.current) void commands.terminalResize(idRef.current, next.cols, next.rows);
      }
      eng.refresh();
    };
    const scheduleFit = () => {
      if (!raf) raf = requestAnimationFrame(doFit);
    };
    const ro = new ResizeObserver(scheduleFit);
    ro.observe(rootRef.current ?? host);

    let unlistenFocus: (() => void) | undefined;
    let focusCancelled = false;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused || disposed) return;
        scheduleFit();
        eng.refresh();
        if (idRef.current) void commands.terminalFocus(idRef.current);
      })
      .then((fn) => {
        if (focusCancelled) fn();
        else unlistenFocus = fn;
      });

    return () => {
      disposed = true;
      focusCancelled = true;
      restartRef.current = null;
      unlistenFocus?.();
      if (raf) cancelAnimationFrame(raf);
      offPress();
      unsubInput();
      unsubKey();
      unsubGone();
      unsubBack();
      offReconnect();
      ro.disconnect();
      const id = idRef.current;
      if (id && exitShownFor !== id) {
        eng.park();
        parked.set(id, { eng, next: feed.next });
        for (const [oldId, old] of parked) {
          if (parked.size <= PARKED_MAX) break;
          parked.delete(oldId);
          old.eng.dispose();
        }
      } else {
        eng.dispose();
      }
      engineRef.current = null;
      feed.release();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workstreamId, shellId]);

  useEffect(() => {
    const eng = engineRef.current;
    if (!eng) return;
    let cancelled = false;
    const raf = requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        if (cancelled) return;
        const g = eng.fit();
        if (idRef.current) void commands.agentResize(idRef.current, g.cols, g.rows);
        eng.refresh();
      }),
    );
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
    };
  }, [dockOpen]);

  // The launch result and hook-reported changes (`/clear`, in-TUI resume) land here.
  useEffect(() => {
    if (!reportedSessionId || reportedSessionId === sessionId) return;
    sessionIdRef.current = reportedSessionId;
    onSessionId?.(reportedSessionId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reportedSessionId, sessionId]);

  useEffect(() => {
    if (hooked || sessionId || !started || !engineResumesById(engine.id)) return;
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine.id, hooked, sessionId, started, workstreamId]);

  // Restart and the bypass toggle resume the conversation when one exists.
  const relaunch = async (bypass: boolean, keep: RelaunchKeep) => {
    const resume = await resolveResume(engine.id, sessionIdRef.current, workstreamId, started);
    restartRef.current?.({ resume, bypass, ...keep });
  };
  // A relaunch kills the terminal, which an adopted agent shares with its parent and every desktop.
  const refuseShared = (what: string): boolean => {
    if (!adopted) return false;
    useToast.getState().show(`${what} not changed`, `Restart the agent to change its ${what.toLowerCase()}.`, {
      tone: "warn",
    });
    return true;
  };

  return (
    <div ref={rootRef} className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface">
      <PtySessionControls
        workstreamId={workstreamId}
        shellId={shellId}
        engineId={engine.id}
        agent={agent}
        onRestart={(keep) => void relaunch(bypassOn, keep)}
        onBypass={(on, keep) => {
          if (!refuseShared("Permission mode")) void relaunch(on, keep);
        }}
        onModelRelaunch={(relaunch) => {
          if (!refuseShared("Model")) restartRef.current?.({ resume: true, bypass: bypassOn, ...relaunch });
        }}
      />
      <div className="relative min-h-0 flex-1">
        <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
        {sizeOwner === false && <TakeSizeButton terminalId={() => idRef.current} />}
        {minting && (
          <div className="absolute inset-0 flex items-center justify-center bg-surface/70 backdrop-blur-[1px]">
            <span className="inline-flex items-center gap-2 text-meta text-ink-dim">
              <Spinner size="xs" />
              Creating session…
            </span>
          </div>
        )}
      </div>
    </div>
  );
}
