import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { RestartIcon, ShieldIcon } from "@/components/icons";
import { Spinner } from "@/components/ui";
import { commands, type TerminalEvent } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { b64ToBytes, strToB64 } from "@/lib/terminal/terminalCodec";
import { releaseChannels } from "@/lib/terminal/channelRelease";
import {
  attachTerminalClipboard,
  useTerminalThemeSync,
  useTerminalZoom,
} from "@/lib/terminal/terminalChrome";
import { createTerminalEngine, type TerminalEngine } from "@/lib/terminal/terminalEngine";
import { xtermThemeFromCss } from "@/lib/terminal/terminalTheme";
import { useScriptsStore } from "@/features/session/scriptsStore";
import { closeAgent, ensureAgent, resolveResume, setAgentBypass } from "./agentRuntime";
import { EngineGlyph } from "@/components/EngineGlyph";
import { AgentStatePill } from "./agentStateUi";
import {
  engineBypassArgs,
  engineMintsViaExec,
  engineResumesById,
  relaunchArgs,
  type ResolvedEngine,
} from "./engines";
import { markAgentStarted, useAgentState } from "./multiplexerStore";

const CAPTURE_INTERVAL_MS = 1500;
const CAPTURE_TRIES = 20;

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
  sessionId?: string;
  started?: boolean;
  bypass?: boolean;
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
  const appliedFontSizeRef = useRef<number>(0);
  const restartRef = useRef<((opts: { resume: boolean; bypass: boolean }) => void) | null>(null);

  const fontSize = useTerminalZoom(engineRef, idRef, appliedFontSizeRef);
  useTerminalThemeSync(engineRef);

  const agent = useAgentState(workstreamId, shellId);
  const state = agent?.state ?? "idle";
  const bypassOn = !!agent?.bypass;
  const started = !!agent?.started;
  const hasBypass = engineBypassArgs(engine.id).length > 0;
  const startedSeed = startedProp ?? !!terminalId;
  const markedStartedRef = useRef(startedSeed);
  const [minting, setMinting] = useState(false);
  const dockOpen = useScriptsStore((s) => !!s.dock[workstreamId]?.open);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const eng = createTerminalEngine({
      fontSize,
      theme: xtermThemeFromCss(),
      cursorBlink: engine.id !== "codex",
    });
    eng.mount(host);
    attachTerminalClipboard(eng);
    engineRef.current = eng;
    appliedFontSizeRef.current = fontSize;
    let lastGrid = eng.fit();
    if (autoFocus) eng.focus();

    let disposed = false;
    const created: Channel<TerminalEvent>[] = [];
    const newChannel = (): Channel<TerminalEvent> => {
      const channel = new Channel<TerminalEvent>();
      created.push(channel);
      channel.onmessage = (ev) => {
        if (disposed) return;
        if (ev.kind === "output") eng.write(b64ToBytes(ev.data));
        else if (ev.kind === "exited") eng.write("\r\n\x1b[2m[agent exited]\x1b[0m\r\n");
      };
      return channel;
    };
    void (async () => {
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      if (disposed) return;
      lastGrid = eng.fit();
      launchedAtRef.current = Date.now();

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
        started: startedSeed,
      });
      if (disposed || !id) return;
      idRef.current = id;
      onTerminalId(id);

      await unwrap(commands.terminalAttach(id, newChannel())).catch(() => false);
      if (disposed) return;
      lastGrid = eng.fit();
      void commands.agentResize(id, lastGrid.cols, lastGrid.rows);
    })();

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
          started: opts.resume,
        });
        if (disposed || !newId) return;
        idRef.current = newId;
        onTerminalId(newId);
        setAgentBypass(workstreamId, shellId, opts.bypass);
        onBypassChange?.(opts.bypass);
        await unwrap(commands.terminalAttach(newId, newChannel())).catch(() => false);
        if (disposed) return;
        lastGrid = eng.fit();
        void commands.agentResize(newId, lastGrid.cols, lastGrid.rows);
      })();
    };

    const unsubInput = eng.onInput((data) => {
      if (idRef.current) void commands.terminalWrite(idRef.current, strToB64(data));
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
      unsubInput();
      unsubKey();
      ro.disconnect();
      eng.dispose();
      engineRef.current = null;
      releaseChannels(created);
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
