import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { useMutation } from "@tanstack/react-query";

import {
  Badge,
  Button,
  CliOutput,
  DropdownSelect,
  ErrorNote,
  Spinner,
  type DropdownOption,
} from "@/components/ui";
import { DownloadIcon, WarningTriangleIcon } from "@/components/icons";
import {
  commands,
  type AgentBundleInstall,
  type AgentBundleReport,
  type AiStatus,
  type CliUpdateResult,
} from "@/lib/bindings";
import { formatRelativeTime, shortSha } from "@/lib/format";
import { unwrap } from "@/lib/ipc";
import {
  invalidateAgentBundle,
  invalidateCliBundleState,
  useAgentBundle,
  useAgentBundleVersions,
  useCliUpdateCheck,
} from "@/lib/queries";

import { CardRow, CARD_ROW_SPAN } from "./cardRow";
import {
  bundleLayout,
  bundlePinState,
  cliFailureText,
  needsSkillsCliUpdate,
  selectedVersion,
  SKILLS_CLI_FLOOR,
  versionOptions,
} from "./bundleVersions";

function InstallOutcome({ result, unpinned }: { result: AgentBundleInstall; unpinned: boolean }) {
  if (!result.ok) {
    return (
      <div className="selectable mt-2 rounded-md border border-bad/30 bg-bad/10 p-3 text-sm">
        <p className="text-bad">
          The reinstall failed
          {result.exitCode != null && <> (exit {result.exitCode})</>}. The CLI did not confirm a
          complete Claude and Codex link set; any personal path collision was left untouched.
        </p>
        <CliOutput text={cliFailureText(result)} />
      </div>
    );
  }
  if (result.source === "fallback") {
    if (unpinned) {
      return (
        <div className="selectable mt-2 rounded-md border border-edge bg-panel-2 p-3 text-sm">
          <p className="text-ink-dim">
            The pin was cleared. No release publishes a bundle for this CLI&rsquo;s own version,
            so it linked Claude and Codex to the repository&rsquo;s current bundle instead.
          </p>
          <CliOutput text={result.stdout} />
        </div>
      );
    }
    return (
      <div className="selectable mt-2 rounded-md border border-warn/30 bg-warn/10 p-3 text-sm">
        <p className="text-warn-ink">
          That version has no published bundle pair, so the CLI linked Claude and Codex to the
          repository fallback instead. What is active no longer matches the version shown.
        </p>
        <CliOutput text={result.stdout} />
      </div>
    );
  }
  if (result.source === "unknown") {
    if (!unpinned) {
      return (
        <div className="selectable mt-2 rounded-md border border-warn/30 bg-warn/10 p-3 text-sm">
          <p className="text-warn-ink">
            The reinstall finished, but its output did not confirm the published release asset
            was used, so treat the installed files as unverified.
          </p>
          <CliOutput text={result.stdout} />
        </div>
      );
    }
    return (
      <div className="selectable mt-2 rounded-md border border-edge bg-panel-2 p-3 text-sm">
        <p className="text-ink-dim">
          The reinstall finished, but its output did not say which source it used. Its own words:
        </p>
        <CliOutput text={result.stdout} />
      </div>
    );
  }
  return (
    <p className="mt-2 text-sm text-ok">
      {unpinned
        ? "Pin cleared — the bundle now tracks the CLI again."
        : "Claude and Codex linked to the published bundle."}
    </p>
  );
}

function RowInstallButton({
  label,
  busy,
  onClick,
}: {
  label: string;
  busy: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      title={label}
      aria-label={label}
      className="flex h-6 w-6 items-center justify-center rounded text-accent enabled:hover:bg-accent/15 disabled:cursor-default"
    >
      {busy ? <Spinner size={12} /> : <DownloadIcon size={14} />}
    </button>
  );
}

function UpdateOutcome({ report, target }: { report: AgentBundleReport; target: string }) {
  if (report.source === "releaseAsset") {
    return (
      <p className="mt-2 text-sm text-ok">
        Updated to {target} — Claude and Codex now point to the published bundle.
      </p>
    );
  }
  return (
    <div className="selectable mt-2 rounded-md border border-warn/30 bg-warn/10 p-3 text-sm">
      <p className="text-warn-ink">
        {report.source === "fallback"
          ? `Claude and Codex were linked to a fallback bundle instead of ${target}.`
          : `Claude and Codex were relinked, but the CLI did not confirm ${target} as the source.`}
      </p>
      <CliOutput text={report.stdout} />
    </div>
  );
}

export function useCliSelfUpdate() {
  return useMutation({
    mutationFn: () => unwrap(commands.updateHyprlayerCli()),
    onSettled: () => invalidateCliBundleState(),
  });
}

export function CliUpdateOutcome({
  result,
  error,
  hint,
}: {
  result: CliUpdateResult | null;
  error: unknown;
  hint?: string | null;
}) {
  return (
    <>
      {error != null && (
        <div className="mt-2">
          <ErrorNote error={error} />
        </div>
      )}
      {result && !result.ok && (
        <div className="selectable mt-2 rounded-md border border-bad/30 bg-bad/10 p-3 text-sm">
          <p className="text-bad">
            The update failed{result.exitCode != null && <> (exit {result.exitCode})</>}.
            {hint && (
              <>
                {" "}
                Run <span className="font-mono text-xs">{hint}</span> in a terminal — your package
                manager may need to prompt.
              </>
            )}
          </p>
          <CliOutput text={cliFailureText(result)} />
        </div>
      )}
      {result?.ok && (
        <p className="mt-2 text-sm text-ok">hyprlayer updated. Re-open Settings to pick it up.</p>
      )}
    </>
  );
}

export function CliUpdateNote({ version }: { version?: string | null }) {
  const check = useCliUpdateCheck();
  const update = useCliSelfUpdate();
  const { current, latest, hint } = check.data ?? {};
  const installed = current ?? version ?? null;
  const stale = needsSkillsCliUpdate(installed) && !update.isSuccess;
  const attempted = update.isPending || update.data != null || update.error != null;

  // the version caption alone just restates the CLI row above it
  if (!latest && !stale && !attempted) return null;

  return (
    <div className="mt-2 text-sm text-ink-dim">
      <div className="flex min-h-6 flex-wrap items-center gap-x-2 gap-y-1">
        <span>hyprlayer {installed ?? `pre-${SKILLS_CLI_FLOOR}`}</span>
        {latest ? (
          <>
            <span className="text-warn-ink">· {latest} available</span>
            <Button variant="primary" busy={update.isPending} onClick={() => update.mutate()}>
              Update
            </Button>
          </>
        ) : (
          stale && <span>· update to {SKILLS_CLI_FLOOR} or newer for current skills</span>
        )}
      </div>

      {!update.isPending && (
        <CliUpdateOutcome result={update.data ?? null} error={update.error} hint={hint} />
      )}
    </div>
  );
}

function UnpairedLayoutNote({ agentTool }: { agentTool?: string | null }) {
  return (
    <div className="mt-1 flex gap-2.5 rounded-md border border-warn/35 bg-warn/8 p-3 text-sm">
      <span className="flex pt-0.5 text-warn-ink">
        <WarningTriangleIcon size={14} />
      </span>
      <div className="min-w-0">
        <p className="text-xs font-semibold text-ink">Bundle versions need Claude and Codex</p>
        <p className="mt-1.5 text-xs leading-relaxed text-pretty text-ink-dim">
          One bundle is linked into <span className="font-mono">~/.claude</span> and{" "}
          <span className="font-mono">~/.codex</span> together, so switching versions needs both
          provisioned. This install reports {agentTool ?? "a single target"} only. Run{" "}
          <span className="font-mono">hyprlayer ai configure</span> to set up the pair.
        </p>
      </div>
    </div>
  );
}

function LegacyLayoutNote({ version }: { version?: string | null }) {
  return (
    <div className="mt-1 flex gap-2.5 rounded-md border border-bad/35 bg-bad/8 p-3 text-sm">
      <span className="flex pt-0.5 text-bad">
        <WarningTriangleIcon size={14} />
      </span>
      <div className="min-w-0">
        <p className="text-xs font-semibold text-ink">Migration required for updates</p>
        <p className="mt-1.5 text-xs leading-relaxed text-pretty text-ink-dim">
          Agent files were copied straight into each target before {SKILLS_CLI_FLOOR}, so
          Hyprlayer cannot update them in place. A newer CLI moves them into one managed
          directory and links them back into <span className="font-mono">~/.claude</span> and{" "}
          <span className="font-mono">~/.codex</span>, so future updates land everywhere at once.
        </p>
        <CliUpdateNote version={version} />
      </div>
    </div>
  );
}

export function AgentBundleSection({ status }: { status: AiStatus | undefined }) {
  const pin = bundlePinState(status);
  const bundle = useAgentBundle({ enabled: status?.installed === true });
  const layout = bundleLayout(status, bundle.data);
  const versioned = layout === "managed";
  const [listWanted, setListWanted] = useState(false);
  const versions = useAgentBundleVersions({ enabled: versioned && listWanted });
  const [choice, setChoice] = useState<string | null>(null);

  const install = useMutation({
    mutationFn: (version: string | null) => unwrap(commands.installAgentBundleVersion(version)),
    onSuccess: (outcome) => {
      if (outcome.ok) setChoice(null);
      invalidateAgentBundle();
    },
    onError: () => invalidateAgentBundle(),
  });

  const update = useMutation({
    mutationFn: (_target: string) => unwrap(commands.updateAgentBundle()),
    onSuccess: () => invalidateAgentBundle(),
    onError: () => invalidateAgentBundle(),
  });

  const listOptions = versionOptions(versions.data);
  const options: DropdownOption[] =
    listOptions.length > 0
      ? listOptions
      : [
          {
            value: "",
            disabled: true,
            label: versions.isError
              ? "Version list unavailable"
              : versions.data
                ? "No versions available"
                : "Loading versions…",
          },
        ];
  const current = selectedVersion(status);
  const value = choice ?? current;
  const pending = choice !== null && choice !== current;
  const busy = install.isPending || update.isPending;
  const lastChecked = bundle.data?.lastChecked;
  const outcome = install.data ?? null;
  const behind =
    !pending && bundle.data?.updateAvailable === true ? (bundle.data.desiredVersion ?? null) : null;
  const sha = bundle.data?.installedSha ? shortSha(bundle.data.installedSha) : null;

  return (
    <CardRow label="Agent bundle" align="center" className="pb-4">
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <div className="flex flex-wrap items-center gap-2">
          {versioned ? (
            <>
              <DropdownSelect
                value={value}
                options={options}
                onChange={(v) => {
                  setChoice(v);
                  install.reset();
                  update.reset();
                }}
                onOpenChange={(open) => open && setListWanted(true)}
                disabled={busy}
                aria-label="Agent bundle version"
                triggerLabel={value || sha || "unknown"}
                className="w-auto px-1.5 py-0.5 text-xs"
              />
              {behind ? (
                <Badge tone="warn">{behind} available</Badge>
              ) : (
                !pending && <Badge tone="ok">Latest</Badge>
              )}
              {pin.kind === "pinned" && pin.drifted && pin.binary && (
                <span className="text-xs text-warn-ink">this CLI ships {pin.binary}</span>
              )}
            </>
          ) : layout === "legacy" ? (
            <Badge tone="bad">Legacy layout</Badge>
          ) : (
            layout === "unpaired" && <Badge tone="warn">Unpaired</Badge>
          )}
        </div>
        {(sha || lastChecked != null) && (
          <span className="font-mono text-xs text-ink-dim">
            {sha}
            {lastChecked != null && <> · checked {formatRelativeTime(lastChecked * 1000)}</>}
          </span>
        )}
      </div>

      <div className="flex items-center gap-1.5">
        {pending && (
          <RowInstallButton label={`Install ${value}`} busy={busy} onClick={() => install.mutate(value)} />
        )}
        {behind && (
          <Button
            variant="primary"
            busy={busy}
            onClick={() => {
              install.reset();
              update.mutate(behind);
            }}
          >
            <DownloadIcon size={13} />
            <span>Update to {behind}</span>
          </Button>
        )}
        {versioned && !pending && pin.kind === "pinned" && (
          <Button
            busy={busy}
            onClick={() => {
              update.reset();
              install.mutate(null);
            }}
          >
            Clear pin
          </Button>
        )}
        <Link to="/agents" className="shrink-0 px-1 text-xs text-accent hover:underline">
          Manage
        </Link>
      </div>

      <div className={CARD_ROW_SPAN}>
        {layout === "legacy" ? (
          <LegacyLayoutNote version={status?.binaryVersion} />
        ) : (
          layout === "unpaired" && <UnpairedLayoutNote agentTool={status?.agentTool} />
        )}

        {busy && (
          <p className="mt-2 flex items-center gap-2 text-sm text-ink-dim">
            <Spinner size={14} />
            Installing the bundle pair and switching the Claude and Codex links…
          </p>
        )}

        {versioned && !busy && versions.isError && (
          <p className="mt-2 text-sm text-ink-dim">
            Could not reach GitHub for the version list, so there is nothing to choose from right
            now. The installed version above is still accurate.
          </p>
        )}
        {versioned && !busy && versions.data && listOptions.length === 0 && (
          <p className="mt-2 text-sm text-ink-dim">
            No recent release publishes an agent bundle for this tool.
          </p>
        )}

        {!busy && install.error != null && (
          <div className="mt-2">
            <ErrorNote error={install.error} />
          </div>
        )}
        {!busy && outcome && (
          <InstallOutcome result={outcome} unpinned={install.variables === null} />
        )}
        {!busy && update.error != null && (
          <div className="mt-2">
            <ErrorNote error={update.error} />
          </div>
        )}
        {!busy && update.data && update.variables != null && (
          <UpdateOutcome report={update.data} target={update.variables} />
        )}
      </div>
    </CardRow>
  );
}
