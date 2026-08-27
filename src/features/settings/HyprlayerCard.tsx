import { useEffect, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

import {
  Badge,
  Card,
  CliOutput,
  ErrorNote,
  IconButton,
  MenuItem,
  Popover,
  Spinner,
} from "@/components/ui";
import { HyprlayerMark } from "@/components/Logo";
import {
  AnytypeIcon,
  CheckIcon,
  DownloadIcon,
  FolderIcon,
  GitBranchIcon,
  NotionIcon,
  ObsidianIcon,
  PencilIcon,
  PersonIcon,
  WarningTriangleIcon,
  WrenchIcon,
  XIcon,
} from "@/components/icons";
import {
  commands,
  type AiStatus,
  type BinaryStatus,
  type Bootstrap,
  type EditorTarget,
} from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import {
  invalidateAgentBundle,
  useAgentBundle,
  useAiStatus,
  useCliUpdateCheck,
  useEditorTargets,
} from "@/lib/queries";
import { useCopyFeedback } from "@/lib/clipboard";
import { IS_MACOS } from "@/lib/platform";

import { AgentBundleSection, CliUpdateOutcome, useCliSelfUpdate } from "./AgentBundleSection";
import { needsCliUpgrade } from "./bundleVersions";
import { CardRow, CARD_ROW_SPAN } from "./cardRow";

function BackendIcon({ kind, size = 14 }: { kind: string; size?: number }) {
  switch (kind) {
    case "git": return <GitBranchIcon size={size} />;
    case "obsidian": return <ObsidianIcon size={size} />;
    case "notion": return <NotionIcon size={size} />;
    case "anytype": return <AnytypeIcon size={size} />;
    default: return null;
  }
}

function StatusPill({ ok, children }: { ok: boolean; children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-edge py-0.5 pl-2 pr-2.5 font-mono text-xs">
      <span className={`h-1.5 w-1.5 rounded-full ${ok ? "bg-ok" : "bg-bad"}`} />
      {children}
    </span>
  );
}

function CliRow({
  binary,
  status,
  upgradeOffered,
}: {
  binary: BinaryStatus | undefined;
  status: AiStatus | undefined;
  upgradeOffered: boolean;
}) {
  const check = useCliUpdateCheck();
  const update = useCliSelfUpdate();
  const reset = update.reset;
  const version = check.data?.current ?? status?.binaryVersion ?? binary?.version ?? null;
  const latest = upgradeOffered ? null : (check.data?.latest ?? null);
  const updated = update.data?.ok === true;
  const failed = update.error != null || update.data?.ok === false;

  useEffect(() => {
    if (!updated) return;
    const timer = window.setTimeout(reset, 2000);
    return () => window.clearTimeout(timer);
  }, [updated, reset]);

  return (
    <CardRow label="CLI">
      <div className="flex min-w-0 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-mono text-xs">{version ?? "—"}</span>
          {latest && <Badge tone="warn">{latest} available</Badge>}
        </div>
        <span className="truncate font-mono text-xs text-ink-dim">
          {binary?.path ?? "not found on PATH"}
        </span>
      </div>

      <div className="flex items-center gap-1.5">
        {!upgradeOffered && (
          <IconButton
            title="Update the hyprlayer CLI"
            aria-label="Update the hyprlayer CLI"
            disabled={update.isPending}
            className={failed ? "text-bad" : updated ? "text-ok" : undefined}
            onClick={() => update.mutate()}
          >
            {update.isPending ? (
              <Spinner size={13} />
            ) : failed ? (
              <WarningTriangleIcon size={14} />
            ) : updated ? (
              <CheckIcon size={14} />
            ) : (
              <DownloadIcon size={13} />
            )}
          </IconButton>
        )}
      </div>

      {!update.isPending && failed && (
        <div className={CARD_ROW_SPAN}>
          <CliUpdateOutcome
            result={update.data ?? null}
            error={update.error}
            hint={check.data?.hint}
          />
        </div>
      )}
    </CardRow>
  );
}

function EditorPicker({ path }: { path: string }) {
  const anchorRef = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const [picked, setPicked] = useState<string | null>(null);
  const editors = useEditorTargets();
  const targets = (editors.data ?? []).filter((t): t is EditorTarget & { openWith: string } =>
    Boolean(t.openWith),
  );

  const pick = (name: string | null, openWith: string | null) => {
    setPicked(name);
    setOpen(false);
    void (openWith ? openPath(path, openWith) : openPath(path)).catch(() => {});
  };

  return (
    <>
      <span ref={anchorRef} className="flex">
        <IconButton
          active={open}
          title={picked ? `Open in ${picked}` : "Open in…"}
          aria-label={picked ? `Open in ${picked}` : "Open in…"}
          onClick={() => setOpen((v) => !v)}
        >
          <PencilIcon size={13} />
        </IconButton>
      </span>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={anchorRef}
        align="end"
        className="min-w-48"
      >
        <MenuItem
          className="flex items-center justify-between gap-2.5"
          onClick={() => pick(null, null)}
        >
          <span>Default app</span>
        </MenuItem>
        {targets.map((t) => (
          <MenuItem
            key={t.id}
            className="flex items-center justify-between gap-2.5"
            onClick={() => pick(t.name, t.openWith)}
          >
            <span>{t.name}</span>
            <span className="font-mono text-2xs text-ink-dim">{t.openWith}</span>
          </MenuItem>
        ))}
      </Popover>
    </>
  );
}

function ConfigRow({ path }: { path: string }) {
  const { copied, trigger } = useCopyFeedback();
  const revealLabel = IS_MACOS ? "Reveal in Finder" : "Show in folder";

  return (
    <CardRow label="Config" align="center">
      <div className="flex min-w-0 items-center gap-2">
        <button
          type="button"
          onClick={() => trigger(path)}
          title="Click to copy"
          className="-ml-1.5 min-w-0 truncate rounded-sm px-1.5 py-0.5 text-left font-mono text-xs text-ink hover:bg-panel-2"
        >
          {path}
        </button>
        {copied && <span className="shrink-0 text-2xs text-ok">Copied</span>}
      </div>

      <div className="flex items-center gap-1.5">
        <IconButton
          title={revealLabel}
          aria-label={revealLabel}
          onClick={() => void revealItemInDir(path).catch(() => {})}
        >
          <FolderIcon size={13} />
        </IconButton>
        <EditorPicker path={path} />
      </div>
    </CardRow>
  );
}

function AgentTargetsRow({ status }: { status: ReturnType<typeof useAiStatus> }) {
  const platforms = status.data?.platforms ?? [];
  const installed = status.data?.installed === true;
  const fallback = status.data?.location ?? null;

  const repair = useMutation({
    mutationFn: () => unwrap(commands.repairAgentBundle()),
    onSettled: () => invalidateAgentBundle(),
  });
  const reset = repair.reset;
  const repaired = repair.data?.ok === true;
  const failure = repair.data != null && !repair.data.ok ? repair.data : null;
  const failed = repair.error != null || failure != null;

  useEffect(() => {
    if (!repaired) return;
    const timer = window.setTimeout(reset, 2000);
    return () => window.clearTimeout(timer);
  }, [repaired, reset]);

  return (
    <CardRow label="Agent targets">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        {status.isLoading ? (
          <span className="flex items-center gap-2 text-sm text-ink-dim">
            <Spinner size={12} /> Ensuring Claude Code and Codex agent links…
          </span>
        ) : !installed ? (
          <>
            <Badge tone="warn">Needs repair</Badge>
            <span className="text-sm text-ink-dim">
              Claude Code and Codex agent links were not fully installed.
            </span>
          </>
        ) : platforms.length > 0 ? (
          platforms.map((platform) => (
            <StatusPill key={platform.id ?? platform.name} ok={platform.installed !== false}>
              {platform.location ?? platform.name}
            </StatusPill>
          ))
        ) : fallback ? (
          <StatusPill ok>{fallback}</StatusPill>
        ) : (
          <span className="text-sm text-ink-dim">{status.data?.agentTool ?? "AI agents"}</span>
        )}
      </div>

      <div className="flex items-center gap-1.5">
        {!status.isLoading && (
          <IconButton
            title="Repair / reinstall"
            aria-label="Repair / reinstall"
            disabled={repair.isPending}
            className={failed ? "text-bad" : repaired ? "text-ok" : undefined}
            onClick={() => repair.mutate()}
          >
            {repair.isPending ? (
              <Spinner size={13} />
            ) : failed ? (
              <WarningTriangleIcon size={14} />
            ) : repaired ? (
              <CheckIcon size={14} />
            ) : (
              <WrenchIcon size={13} />
            )}
          </IconButton>
        )}
      </div>

      {(status.error != null || failed) && (
        <div className={CARD_ROW_SPAN}>
          {status.error != null && <ErrorNote error={status.error} />}
          {repair.error != null && (
            <div className="mt-1">
              <ErrorNote error={repair.error} onDismiss={reset} />
            </div>
          )}
          {failure && (
            <div className="selectable relative mt-1 rounded-md border border-bad/30 bg-bad/10 p-3 text-sm">
              <button
                type="button"
                onClick={reset}
                aria-label="Dismiss error"
                className="absolute right-1.5 top-1.5 flex h-6 w-6 items-center justify-center rounded text-bad/70 hover:bg-bad/15 hover:text-bad"
              >
                <XIcon size={11} />
              </button>
              <p className="pr-5 text-bad">
                The repair failed{failure.exitCode != null && <> (exit {failure.exitCode})</>}. The
                Claude and Codex links were left as they were.
              </p>
              <CliOutput text={failure.stderr || failure.stdout} />
            </div>
          )}
        </div>
      )}
    </CardRow>
  );
}

export function HyprlayerCard({ data }: { data: Bootstrap }) {
  const aiStatus = useAiStatus();
  const bundle = useAgentBundle({ enabled: aiStatus.data?.installed === true });
  const config = data.config;
  const upgradeOffered = needsCliUpgrade(aiStatus.data, bundle.data);

  return (
    <Card className="p-0">
      <header className="mb-1 flex items-center justify-between gap-4 border-b border-edge px-4 py-3.5">
        <div className="flex items-center gap-2.5">
          <HyprlayerMark size={16} />
          <span className="text-sm font-semibold">Hyprlayer</span>
        </div>
        {config.state === "ready" && (
          <div className="flex items-center gap-3.5 text-xs text-ink-dim">
            <span className="inline-flex items-center gap-1.5">
              <BackendIcon kind={config.backendKind} />
              <span className="font-mono">{config.backendKind}</span>
              {!config.configured && <Badge tone="warn">incomplete</Badge>}
            </span>
            <span className="inline-flex items-center gap-1.5">
              <PersonIcon size={14} />
              <span className="font-mono">{config.user || "—"}</span>
            </span>
          </div>
        )}
      </header>

      <CliRow
        binary={data.binaries.find((b) => b.kind === "hyprlayer")}
        status={aiStatus.data}
        upgradeOffered={upgradeOffered}
      />

      {config.state === "ready" && <ConfigRow path={config.path} />}
      {config.state === "missing" && (
        <CardRow label="Config">
          <div className="min-w-0 space-y-1.5">
            <Badge tone="warn">Not set up yet</Badge>
            <p className="text-sm text-ink-dim">
              No config found at <span className="selectable font-mono text-xs">{config.path}</span>
              . Add your first project from the dashboard.
            </p>
          </div>
        </CardRow>
      )}
      {config.state === "error" && (
        <CardRow label="Config">
          <div className="min-w-0">
            <ErrorNote error={{ message: config.message, remedy: config.remedy }} />
          </div>
        </CardRow>
      )}

      <AgentTargetsRow status={aiStatus} />

      {aiStatus.data?.installed && <AgentBundleSection status={aiStatus.data} />}
    </Card>
  );
}
