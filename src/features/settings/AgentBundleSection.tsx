import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { useMutation } from "@tanstack/react-query";

import {
  Button,
  CliOutput,
  DropdownSelect,
  ErrorNote,
  Spinner,
  type DropdownOption,
} from "@/components/ui";
import { DownloadIcon } from "@/components/icons";
import {
  commands,
  type AgentBundleInstall,
  type AgentBundleReport,
  type AiStatus,
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

import { bundlePinState, selectedVersion, versionOptions } from "./bundleVersions";

function InstallOutcome({ result, unpinned }: { result: AgentBundleInstall; unpinned: boolean }) {
  if (!result.ok) {
    return (
      <div className="selectable mt-2 rounded-md border border-bad/30 bg-bad/10 p-3 text-sm">
        <p className="text-bad">
          The reinstall failed
          {result.exitCode != null && <> (exit {result.exitCode})</>}. The CLI may have stopped
          partway through, so the bundle on disk is not guaranteed untouched.
        </p>
        {result.backupPath && (
          <p className="mt-1 text-ink-dim">
            Your pre-install files were kept at{" "}
            <span className="font-mono text-xs">{result.backupPath}</span> — until the next
            install attempt, which replaces that copy.
          </p>
        )}
        <CliOutput text={result.stderr || result.stdout} />
      </div>
    );
  }
  if (result.source === "fallback") {
    if (unpinned) {
      return (
        <div className="selectable mt-2 rounded-md border border-edge bg-panel-2 p-3 text-sm">
          <p className="text-ink-dim">
            The pin was cleared. No release publishes a bundle for this CLI&rsquo;s own version,
            so it installed the repository&rsquo;s current files instead — the same bundle an
            unpinned pre-1.6.0 install uses.
          </p>
          <CliOutput text={result.stdout} />
        </div>
      );
    }
    return (
      <div className="selectable mt-2 rounded-md border border-warn/30 bg-warn/10 p-3 text-sm">
        <p className="text-warn">
          That version has no published bundle, so the CLI installed an unversioned copy from the
          repository instead. What is on disk no longer matches the version shown.
        </p>
        {result.backupPath && (
          <p className="mt-1 text-ink-dim">
            Your previous bundle was backed up to{" "}
            <span className="font-mono text-xs">{result.backupPath}</span>. Reinstalling another
            version will not undo this — the installer reads these files as your own edits and
            keeps them.
          </p>
        )}
        <CliOutput text={result.stdout} />
      </div>
    );
  }
  if (result.source === "unknown") {
    if (!unpinned) {
      return (
        <div className="selectable mt-2 rounded-md border border-warn/30 bg-warn/10 p-3 text-sm">
          <p className="text-warn">
            The reinstall finished, but its output did not confirm the published release asset
            was used, so treat the installed files as unverified.
          </p>
          {result.backupPath && (
            <p className="mt-1 text-ink-dim">
              Your previous bundle was backed up to{" "}
              <span className="font-mono text-xs">{result.backupPath}</span>.
            </p>
          )}
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
        : "Bundle reinstalled from the published release."}
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

function listOf(parts: string[]): string {
  if (parts.length <= 1) return parts[0] ?? "";
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

function UpdateOutcome({ report, target }: { report: AgentBundleReport; target: string }) {
  const changed = listOf(
    [
      report.agents.length > 0
        ? `${report.agents.length} agent file${report.agents.length === 1 ? "" : "s"}`
        : null,
      report.settingsChanged ? "settings.json" : null,
      report.skillsChanged > 0
        ? `${report.skillsChanged} skill file${report.skillsChanged === 1 ? "" : "s"}`
        : null,
    ].filter((part): part is string => part !== null),
  );
  if (changed === "") {
    return (
      <p className="mt-2 text-sm text-ok">Now on {target} — the files on disk already matched it.</p>
    );
  }
  return (
    <div
      className={`selectable mt-2 rounded-md border p-3 text-sm ${
        report.settingsChanged ? "border-warn/30 bg-warn/10" : "border-edge bg-panel-2"
      }`}
    >
      <p className={report.settingsChanged ? "text-warn" : "text-ok"}>
        Updated to {target} — replaced {changed}.
      </p>
      <p className="mt-1 text-ink-dim">
        Your previous copies are at{" "}
        <span className="font-mono text-xs">{report.backupPath}</span> — until the next install,
        which replaces that copy.
      </p>
    </div>
  );
}

function CliUpdateNote() {
  const check = useCliUpdateCheck();
  const update = useMutation({
    mutationFn: () => unwrap(commands.updateHyprlayerCli()),
    onSettled: () => invalidateCliBundleState(),
  });
  const { current, latest, hint } = check.data ?? {};
  const result = update.data ?? null;

  return (
    <div className="mt-2 text-ink-dim">
      <div className="flex min-h-6 flex-wrap items-center gap-x-2 gap-y-1">
        <span>hyprlayer {current ?? "pre-1.6.0"}</span>
        {latest ? (
          <>
            <span className="text-warn">· {latest} available</span>
            <Button
              variant="primary"
              busy={update.isPending}
              onClick={() => update.mutate()}
            >
              Update
            </Button>
          </>
        ) : (
          !update.isSuccess && <span>· update to 1.6.0 or newer for current skills</span>
        )}
      </div>

      {!update.isPending && update.error != null && (
        <div className="mt-2">
          <ErrorNote error={update.error} />
        </div>
      )}
      {!update.isPending && result && !result.ok && (
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
          <CliOutput text={result.stderr || result.stdout} />
        </div>
      )}
      {!update.isPending && result?.ok && (
        <p className="mt-2 text-sm text-ok">hyprlayer updated. Re-open Settings to pick it up.</p>
      )}
    </div>
  );
}

export function AgentBundleSection({ status }: { status: AiStatus | undefined }) {
  const pin = bundlePinState(status);
  const bundle = useAgentBundle({ enabled: status?.installed === true });
  const manageable = bundle.data?.manageable === true;
  const versioned = manageable && pin.kind !== "unavailable";
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

  return (
    <div className="mt-3 border-t border-edge/50 pt-3 text-sm">
      <div className="flex min-h-6 items-center justify-between gap-2">
        <span className="flex min-w-0 flex-wrap items-center gap-x-1 text-ink-dim">
          Agent bundle
          {versioned ? (
            <>
              {" · "}
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
                triggerLabel={
                  value ||
                  (bundle.data?.installedSha ? shortSha(bundle.data.installedSha) : "unknown")
                }
                className="w-auto border-transparent bg-transparent px-1 py-0 text-xs text-ink hover:border-edge hover:bg-panel-2"
              />
              {pending && (
                <RowInstallButton
                  label={`Install ${value}`}
                  busy={busy}
                  onClick={() => install.mutate(value)}
                />
              )}
              {behind && (
                <>
                  {" · "}
                  <span className="text-warn">{behind} available</span>
                  <RowInstallButton
                    label={`Update to ${behind}`}
                    busy={busy}
                    onClick={() => {
                      install.reset();
                      update.mutate(behind);
                    }}
                  />
                </>
              )}
              {pin.kind === "pinned" && pin.drifted && pin.binary && (
                <span className="text-warn">{" · "}this CLI ships {pin.binary}</span>
              )}
            </>
          ) : (
            bundle.data?.installedSha && (
              <>
                {" · "}
                <span className="font-mono text-xs">{shortSha(bundle.data.installedSha)}</span>
              </>
            )
          )}
          {lastChecked != null && <> · checked {formatRelativeTime(lastChecked * 1000)}</>}
        </span>
        <Link to="/agents" className="shrink-0 text-xs text-accent hover:underline">
          Manage Agents
        </Link>
      </div>

      {manageable && pin.kind === "unavailable" && <CliUpdateNote />}

      {versioned && !pending && pin.kind === "pinned" && (
        <div className="mt-2">
          <Button
            busy={busy}
            onClick={() => {
              update.reset();
              install.mutate(null);
            }}
          >
            Clear pin
          </Button>
        </div>
      )}

      {busy && (
        <p className="mt-2 flex items-center gap-2 text-ink-dim">
          <Spinner size={14} />
          Reinstalling the bundle — this rewrites ~/.claude and takes a few seconds.
        </p>
      )}

      {versioned && !busy && versions.isError && (
        <p className="mt-2 text-ink-dim">
          Could not reach GitHub for the version list, so there is nothing to choose from right
          now. The installed version above is still accurate.
        </p>
      )}
      {versioned && !busy && versions.data && listOptions.length === 0 && (
        <p className="mt-2 text-ink-dim">
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
  );
}
