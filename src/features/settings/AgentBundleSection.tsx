/**
 * The AI Agents card's agent-bundle section: which bundle is installed, whether
 * it is pinned, when it was last checked, and a picker to change it.
 *
 * One row, not two. The bundle has a single identity, and since CLI 1.6.0 that
 * identity is a version — so the version stands where the install SHA used to,
 * and is itself the control: click it to pick another. The SHA is still what
 * the freshness probe compares, so it stays the row's subject for a CLI that
 * reports no version at all, for a bundle installed before versioned assets
 * existed, and for a tool this app cannot manage.
 *
 * Renders only inside the card's `installed` branch — an unconfigured machine
 * has no bundle to describe. The derivations it switches on are in
 * `bundleVersions.ts`; this file is the surface.
 */
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
import { commands, type AgentBundleInstall, type AiStatus } from "@/lib/bindings";
import { formatRelativeTime, shortSha } from "@/lib/format";
import { unwrap } from "@/lib/ipc";
import { invalidateAgentBundle, useAgentBundle, useAgentBundleVersions } from "@/lib/queries";

import { bundlePinState, selectedVersion, versionOptions } from "./bundleVersions";

/**
 * What one finished install run should say.
 *
 * The fallback case is the reason this is not a toast. `reinstall --version`
 * exits 0 and records the requested version even when its source chain fell
 * through to master's tree, so the app's own state — and `ai status` — agree
 * with the picker while `~/.claude` holds something else. The only signal is
 * the CLI's stdout, so when it says a fallback answered, the run is reported
 * as a problem with the backup path attached, not as a success.
 *
 * `unpinned` splits every verdict: for `--unpin` a fallback is the CLI
 * legitimately installing the repository tree because its own version has no
 * published asset (a source-built or nightly binary) — the same bundle an
 * unpinned pre-1.6.0 install uses, not a corrupted one — and there is no
 * "version shown" left to mismatch.
 */
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
    // For a picked version this is a caution, not trivia: the wording drift
    // that lands here is also the decay mode of the fallback tripwire above,
    // so "could not confirm" must not read calmer than "confirmed bad".
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

export function AgentBundleSection({ status }: { status: AiStatus | undefined }) {
  const pin = bundlePinState(status);
  const bundle = useAgentBundle({ enabled: status?.installed === true });
  // The freshness probe's `manageable` (an installed Claude Code) gates every
  // interactive control here, not just `installed`: `ai versions` follows the
  // CLI's *configured* harness, so on any other tool the picker would offer a
  // list whose every install the backend refuses — a dead-end control, plus a
  // GitHub-bound spawn for a machine the bundle surfaces otherwise leave
  // alone. Undefined-while-loading reads as false, so the row degrades to the
  // read-only SHA it showed before versions existed.
  const manageable = bundle.data?.manageable === true;
  const versioned = manageable && pin.kind !== "unavailable";
  // The list is fetched on first picker interaction, not on mount: it spawns
  // a CLI process that can reach the GitHub releases API, and most Settings
  // visits never open the menu. The row's headline needs no list — it reads
  // `ai status`, which is local and already in hand.
  const [listWanted, setListWanted] = useState(false);
  const versions = useAgentBundleVersions({ enabled: versioned && listWanted });
  const [choice, setChoice] = useState<string | null>(null);

  const install = useMutation({
    /** `null` clears the pin (`reinstall --unpin`). */
    mutationFn: (version: string | null) => unwrap(commands.installAgentBundleVersion(version)),
    onSuccess: (outcome) => {
      // A CLI failure arrives as data (`ok: false`), not an error, so only a
      // run that actually installed clears the selection — otherwise the
      // Install button must stand for a retry.
      if (outcome.ok) setChoice(null);
      invalidateAgentBundle();
    },
    onError: () => invalidateAgentBundle(),
  });

  const listOptions = versionOptions(versions.data);
  // The menu needs a body before the lazily-fetched list arrives (and when it
  // cannot arrive): one disabled row saying why, never a selectable invention
  // — options a user can actually pick still come only from `versionOptions`.
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
  // The install affordance arms only while the selection differs from what
  // the row rests on — the pin, or the installed version when unpinned — so
  // picking back the current version disarms it. Comparing against the PIN
  // alone is the bug this replaces: on an unpinned machine the pin is null,
  // no choice ever equals it, and the icon could never disarm. Accepted gap:
  // an unpinned machine cannot pin the version it is already on through the
  // picker — that selection reads as "no change", and offering an install
  // there advertises an action as though something needed doing.
  const current = selectedVersion(status);
  const value = choice ?? current;
  const pending = choice !== null && choice !== current;
  const busy = install.isPending;
  const lastChecked = bundle.data?.lastChecked;
  const outcome = install.data ?? null;

  return (
    <div className="mt-3 border-t border-edge/50 pt-3 text-sm">
      {/* min-h-6 reserves the row's tallest occupant — the install icon's
          inline tier (h-6, 24px) — up front. Without it the ~20px text row
          stretches 4px when a pick arms the icon and collapses back when the
          selection returns to the current version. */}
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
                  // A fresh pick supersedes the previous run's report and
                  // error — leaving them up reads as describing this pick.
                  install.reset();
                }}
                onOpenChange={(open) => open && setListWanted(true)}
                disabled={busy}
                aria-label="Agent bundle version"
                // The identifier itself is the control: styled as the row's
                // own text until hovered, so it reads where the SHA read
                // rather than as a form field parked in a sentence. The menu
                // keeps the full labels; `triggerLabel` holds the line to the
                // bare version — or to the install SHA for a 1.6.0 CLI over a
                // bundle installed before versioned assets existed, where the
                // SHA is still the bundle's only identity.
                triggerLabel={
                  value ||
                  (bundle.data?.installedSha ? shortSha(bundle.data.installedSha) : "unknown")
                }
                className="w-auto border-transparent bg-transparent px-1 py-0 text-xs text-ink hover:border-edge hover:bg-panel-2"
              />
              {/* The install affordance rides the row instead of standing as a
                  labeled button below it: it only exists after an explicit
                  pick, and the picked version is already the trigger text two
                  characters to its left, so a label would repeat it. ADR-0006
                  inline icon tier (h-6 w-6 rounded); accent-tinted because it
                  commits a ~/.claude rewrite — a semantic tint, outside the
                  neutral hover split by design. */}
              {pending && (
                <button
                  type="button"
                  onClick={() => install.mutate(value)}
                  disabled={busy}
                  title={`Install ${value}`}
                  aria-label={`Install ${value}`}
                  className="flex h-6 w-6 items-center justify-center rounded text-accent enabled:hover:bg-accent/15 disabled:cursor-default"
                >
                  {busy ? <Spinner size={12} /> : <DownloadIcon size={14} />}
                </button>
              )}
              {/* Only the drift is worth words. That a version is pinned is
                  already what the row shows — picking one pins it — and the
                  Clear pin button below says so in a way the user can act on.
                  A pin that disagrees with the binary is the exception: it is
                  the state users get stuck in, reinstalling the pinned bundle
                  silently on every CLI upgrade, and nothing else on the row
                  reveals it. */}
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
          {/* "checked", not "updated": the CLI writes this when its 24h
              auto-check LOOKS for a new bundle, and `record_install` clears
              it on an actual install — so labelling it "updated" reports the
              one moment it is guaranteed not to be. Same wording as the
              Agents header chip. */}
          {lastChecked != null && <> · checked {formatRelativeTime(lastChecked * 1000)}</>}
        </span>
        <Link to="/agents" className="shrink-0 text-xs text-accent hover:underline">
          Manage Agents
        </Link>
      </div>

      {/* Only for a manageable tool: on any other, the CLI being 1.6.0 would
          not make the picker appear, so the upgrade hint would be a lie. */}
      {manageable && pin.kind === "unavailable" && (
        <p className="mt-2 text-ink-dim">
          Update the hyprlayer CLI to 1.6.0 or newer to pin the bundle to a specific version.
        </p>
      )}

      {versioned && !pending && pin.kind === "pinned" && (
        <div className="mt-2">
          <Button busy={busy} onClick={() => install.mutate(null)}>
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

      {/* The list failing must not take the row with it: the version above is
          read from `ai status`, which is local and already loaded. Offline
          with a cold CLI cache is the case that lands here. */}
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
    </div>
  );
}
