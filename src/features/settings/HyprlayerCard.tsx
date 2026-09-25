import { useEffect, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

import {
  Badge,
  Card,
  CliOutput,
  cx,
  ErrorNote,
  IconButton,
  MenuItem,
  Popover,
  Spinner,
} from "@/components/ui";
import { Icon } from "@/components/Icon";
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
  useConnectionStatus,
  useEditorTargets,
} from "@/lib/queries";
import { useCopyFeedback } from "@/lib/clipboard";
import { IS_MACOS } from "@/lib/platform";

import { AgentBundleSection, CliUpdateOutcome, useCliSelfUpdate } from "./AgentBundleSection";
import { bundleLayout, cliFailureText } from "./bundleVersions";
import { CardRow, CARD_ROW_SPAN } from "./cardRow";

// Inline tier (ADR-0006): these sit inside the card's rows.
const ROW_BUTTON = "h-6 w-6";

function StatusPill({ ok, children }: { ok: boolean; children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-edge py-0.5 pl-2 pr-2.5 text-meta">
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
  // the migration note names the newer version itself; don't badge it twice
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
          <span className="text-meta">{version ?? "—"}</span>
          {latest && <Badge tone="warn">{latest} available</Badge>}
        </div>
        <span className="truncate text-meta text-ink-dim">
          {binary?.path ?? "not found on PATH"}
        </span>
      </div>

      <div className="flex items-center gap-1.5">
        <IconButton
          title="Update the hyprlayer CLI"
          aria-label="Update the hyprlayer CLI"
          disabled={update.isPending}
          className={cx(ROW_BUTTON, failed ? "text-bad" : updated ? "text-ok" : undefined)}
          onClick={() => update.mutate()}
        >
          {update.isPending ? (
            <Spinner size="sm" />
          ) : failed ? (
            <Icon name="failed" size="sm" />
          ) : updated ? (
            <Icon name="check" size="sm" />
          ) : (
            <Icon name="download" />
          )}
        </IconButton>
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

function EditorPicker({ path, disabledReason }: { path: string; disabledReason: string | null }) {
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
          className={ROW_BUTTON}
          disabled={disabledReason != null}
          title={disabledReason ?? (picked ? `Edit config in ${picked}` : "Edit config")}
          aria-label="Edit config"
          onClick={() => setOpen((v) => !v)}
        >
          <Icon name="edit" />
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
            <span className="text-meta text-ink-dim">{t.openWith}</span>
          </MenuItem>
        ))}
      </Popover>
    </>
  );
}

function ConfigRow({ path }: { path: string }) {
  const { copied, trigger } = useCopyFeedback();
  const revealLabel = IS_MACOS ? "Reveal in Finder" : "Show in folder";
  // The path is the server's; revealing or opening it would act on this machine.
  const remote = useConnectionStatus().data?.host ?? null;
  const disabledReason = remote ? `The config is on ${remote.label || remote.destination}` : null;

  return (
    <CardRow label="Config" align="center">
      <div className="flex min-w-0 items-center gap-2">
        <button
          type="button"
          onClick={() => trigger(path)}
          title="Click to copy"
          className="-ml-1.5 min-w-0 truncate rounded-sm px-1.5 py-0.5 text-left text-meta text-ink hover:bg-panel-2"
        >
          {path}
        </button>
        {copied && <span className="shrink-0 text-ui-sm text-ok">Copied</span>}
      </div>

      <div className="flex items-center gap-1.5">
        <IconButton
          className={ROW_BUTTON}
          disabled={disabledReason != null}
          title={disabledReason ?? revealLabel}
          aria-label={revealLabel}
          onClick={() => void revealItemInDir(path).catch(() => {})}
        >
          <Icon name="folderOpen" />
        </IconButton>
        <EditorPicker path={path} disabledReason={disabledReason} />
      </div>
    </CardRow>
  );
}

function AgentTargetsRow({ status }: { status: ReturnType<typeof useAiStatus> }) {
  const platforms = status.data?.platforms ?? [];
  const installed = status.data?.installed === true;
  const fallback = status.data?.location ?? null;
  const broken = platforms
    .filter((p) => p.installed === false)
    .map((p) => p.name ?? p.id)
    .filter((name): name is string => Boolean(name));

  const repair = useMutation({
    mutationFn: () => unwrap(commands.repairAgentBundle()),
    onSettled: () => invalidateAgentBundle(),
  });
  const reset = repair.reset;
  const repaired = repair.data?.ok === true;
  const failure = repair.data != null && !repair.data.ok ? repair.data : null;
  const failed = repair.error != null || failure != null;
  const resolved = repair.data?.resolved ?? null;
  const moved = (resolved?.replaced.length ?? 0) + (resolved?.backedUp.length ?? 0) > 0;

  useEffect(() => {
    // a repair that touched the user's files keeps its receipt until the next run
    if (!repaired || moved) return;
    const timer = window.setTimeout(reset, 2000);
    return () => window.clearTimeout(timer);
  }, [repaired, moved, reset]);

  return (
    <CardRow label="Agent targets">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        {status.isLoading ? (
          <span className="flex items-center gap-2 text-body text-ink-dim">
            <Spinner size="xs" /> Ensuring Claude Code and Codex agent links…
          </span>
        ) : !installed ? (
          <>
            <span className="rounded-control bg-bad/10 px-1.5 py-0.5 text-meta font-semibold text-bad-ink">
              NEEDS REPAIR
            </span>
            <span className="text-ui text-ink-2">
              {broken.length > 0 ? broken.join(" and ") : "Claude Code and Codex"} agent links were
              not fully installed.
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
          <span className="text-body text-ink-dim">{status.data?.agentTool ?? "AI agents"}</span>
        )}
      </div>

      <div className="flex items-center gap-1.5">
        {!status.isLoading && (
          <IconButton
            title="Repair / reinstall"
            aria-label="Repair / reinstall"
            disabled={repair.isPending}
            className={cx(ROW_BUTTON, failed ? "text-bad" : repaired ? "text-ok" : undefined)}
            onClick={() => repair.mutate()}
          >
            {repair.isPending ? (
              <Spinner size="sm" />
            ) : failed ? (
              <Icon name="failed" size="sm" />
            ) : repaired ? (
              <Icon name="check" size="sm" />
            ) : (
              <Icon name="tool" />
            )}
          </IconButton>
        )}
      </div>

      {repaired && moved && resolved && (
        <div className={CARD_ROW_SPAN}>
          <div className="mt-1 rounded-md border border-edge bg-panel-2 p-3 text-body">
            <p className="text-ok">Relinked Claude and Codex.</p>
            {resolved.replaced.length > 0 && (
              <p className="mt-1.5 text-ui text-ink-dim">
                Removed {resolved.replaced.length} stale{" "}
                {resolved.replaced.length === 1 ? "copy" : "copies"} that already matched the
                bundle.
              </p>
            )}
            {resolved.backedUp.length > 0 && (
              <p className="mt-1.5 text-ui text-ink-dim">
                Kept {resolved.backedUp.length} edited{" "}
                {resolved.backedUp.length === 1 ? "copy" : "copies"} — moved aside as{" "}
                <span className="font-mono">.bak-</span>, not deleted.
              </p>
            )}
          </div>
        </div>
      )}

      {(status.error != null || failed) && (
        <div className={CARD_ROW_SPAN}>
          {status.error != null && <ErrorNote error={status.error} />}
          {repair.error != null && (
            <div className="mt-1">
              <ErrorNote error={repair.error} onDismiss={reset} />
            </div>
          )}
          {failure && (
            <div className="selectable relative mt-1 rounded-md border border-bad/30 bg-bad/10 p-3 text-body">
              <button
                type="button"
                onClick={reset}
                aria-label="Dismiss error"
                className="absolute right-1.5 top-1.5 flex h-6 w-6 items-center justify-center rounded text-bad/70 hover:bg-bad/15 hover:text-bad"
              >
                <Icon name="close" size="xs" />
              </button>
              <p className="pr-5 text-bad">
                The repair failed{failure.exitCode != null && <> (exit {failure.exitCode})</>}. The
                Claude and Codex links were left as they were.
              </p>
              <CliOutput text={cliFailureText(failure)} />
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
  const upgradeOffered = bundleLayout(aiStatus.data, bundle.data) === "legacy";

  return (
    <Card className="divide-y divide-edge-2 p-0">
      <header className="flex items-center gap-2.5 px-4 py-3.5">
        <span className="h-2 w-2 shrink-0 bg-ink" aria-hidden="true" />
        <span className="text-body font-semibold text-ink">Hyprlayer</span>
        {config.state === "ready" && (
          <span className="flex min-w-0 items-center gap-2 text-meta text-ink-dim">
            <span>{config.backendKind}</span>
            {!config.configured && <Badge tone="warn">incomplete</Badge>}
            <span aria-hidden="true">·</span>
            <span className="truncate">{config.user || "—"}</span>
          </span>
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
            <p className="text-body text-ink-dim">
              No config found at <span className="selectable text-meta">{config.path}</span>
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
