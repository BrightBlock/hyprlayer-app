import { useState } from "react";

import type { AppSettings, GitSettings } from "@/lib/bindings";
import { useGithubUsername, useSetAppSettings } from "@/lib/queries";
import { BranchPrefixEditor } from "@/components/BranchPrefixEditor";
import { SETTINGS_CARD_DESCRIPTION, SettingsCardTitle } from "@/components/SettingsCard";
import { Card, cx, ErrorNote, SettingRow, TextInput, Toggle } from "@/components/ui";

const DEFAULT_GIT: Required<GitSettings> = {
  branchPrefix: null,
  prefixes: null,
  deleteBranchOnArchive: true,
  archiveOnMerge: false,
  automerge: false,
  pruneIgnoredOnArchive: true,
};

const DEFAULT_PREFIX = "hyprlayer/";
const SAMPLE_SLUG = "add-login";

type PrefixMode = "default" | "github" | "custom";

export function GitSettings({ settings }: { settings: AppSettings }) {
  const raw = settings.git ?? {};
  const git: Required<GitSettings> = {
    branchPrefix: raw.branchPrefix ?? DEFAULT_GIT.branchPrefix ?? null,
    prefixes: raw.prefixes ?? DEFAULT_GIT.prefixes ?? null,
    deleteBranchOnArchive: raw.deleteBranchOnArchive ?? DEFAULT_GIT.deleteBranchOnArchive,
    archiveOnMerge: raw.archiveOnMerge ?? DEFAULT_GIT.archiveOnMerge,
    automerge: raw.automerge ?? DEFAULT_GIT.automerge,
    pruneIgnoredOnArchive:
      raw.pruneIgnoredOnArchive ?? DEFAULT_GIT.pruneIgnoredOnArchive,
  };
  const save = useSetAppSettings();
  const username = useGithubUsername();

  const initialCustom =
    git.branchPrefix && git.branchPrefix !== `${username.data ?? ""}/`
      ? git.branchPrefix
      : "feature/";
  const [customPrefix, setCustomPrefix] = useState(initialCustom);

  const effectivePrefix = git.branchPrefix ?? DEFAULT_PREFIX;
  const ghPrefix = username.data ? `${username.data}/` : null;

  const mode: PrefixMode =
    git.branchPrefix == null
      ? "default"
      : ghPrefix && git.branchPrefix === ghPrefix
        ? "github"
        : "custom";

  function patch(next: Partial<GitSettings>) {
    save.mutate({ ...settings, git: { ...git, ...next } });
  }

  function pickMode(m: PrefixMode) {
    if (m === "default") patch({ branchPrefix: null });
    else if (m === "github" && ghPrefix) patch({ branchPrefix: ghPrefix });
    else patch({ branchPrefix: customPrefix });
  }

  const radio = (m: PrefixMode, label: React.ReactNode) => (
    <button
      type="button"
      role="radio"
      aria-checked={mode === m}
      onClick={() => pickMode(m)}
      className="flex items-center gap-2 text-left text-body text-ink"
    >
      <span
        className={`flex h-4 w-4 items-center justify-center rounded-full border ${
          mode === m ? "border-accent" : "border-edge"
        }`}
      >
        {mode === m && <span className="h-2 w-2 rounded-full bg-accent" />}
      </span>
      {label}
    </button>
  );

  return (
    <Card>
      <SettingsCardTitle>Git</SettingsCardTitle>
      <p className={cx("mt-1", SETTINGS_CARD_DESCRIPTION)}>
        Branch naming, and what archiving or merging does to a workstream's worktree.
      </p>

      {save.isError && (
        <div className="mt-3">
          <ErrorNote error={save.error} onDismiss={() => save.reset()} />
        </div>
      )}

      <div className="mt-3 divide-y divide-edge/50">
        <div className="py-3.5">
          <p className="text-body text-ink">Branch name prefix</p>
          <div className="mt-3 space-y-2.5">
            {radio("default", <span>Default ({DEFAULT_PREFIX})</span>)}
            {radio(
              "github",
              ghPrefix ? (
                <span>GitHub username ({username.data})</span>
              ) : (
                <span className="text-ink-dim">
                  GitHub username{" "}
                  {username.isLoading ? "(detecting…)" : "(unavailable, run gh auth login)"}
                </span>
              ),
            )}
            {radio("custom", <span>Custom</span>)}
            <div className="pl-6">
              <TextInput
                value={customPrefix}
                placeholder="feature/"
                onFocus={() => mode !== "custom" && pickMode("custom")}
                onChange={(e) => setCustomPrefix(e.target.value)}
                onBlur={() => mode === "custom" && patch({ branchPrefix: customPrefix })}
                onKeyDown={(e) => {
                  if (e.key === "Enter") patch({ branchPrefix: customPrefix });
                }}
                className="max-w-xs"
              />
              <p className="mt-1.5 text-ui text-ink-dim">
                Preview:{" "}
                <span className="font-mono text-ink">
                  {effectivePrefix}
                  {SAMPLE_SLUG}
                </span>
              </p>
            </div>
          </div>
        </div>

        <div className="py-3.5">
          <BranchPrefixEditor
            title={<p className="text-body text-ink">Branch types</p>}
            description="Customize the branch types for what you're working on. Picking one replaces the prefix above for that workstream."
            value={git.prefixes ?? []}
            onChange={(next) => patch({ prefixes: next.length ? next : null })}
            disabled={save.isPending}
            blockReseed={save.isPending || save.isError}
            inheritedNote="Built-in types"
          />
        </div>

        <SettingRow
          title="Delete branch on archive"
          control={
            <Toggle
              aria-label="Delete branch on archive"
              checked={git.deleteBranchOnArchive}
              onChange={(v) => patch({ deleteBranchOnArchive: v })}
            />
          }
        >
          Only branches Hyprlayer created.
        </SettingRow>

        <SettingRow
          title="Archive on merge"
          control={
            <Toggle
              aria-label="Archive on merge"
              checked={git.archiveOnMerge}
              onChange={(v) => patch({ archiveOnMerge: v })}
            />
          }
        >
          Triggered by the pull request merging, not a local merge.
        </SettingRow>

        <SettingRow
          title="Automerge"
          control={
            <Toggle
              aria-label="Automerge"
              checked={git.automerge}
              onChange={(v) => patch({ automerge: v })}
            />
          }
        >
          Merges the pull request once required checks pass. The repo must allow auto-merge.
        </SettingRow>

        <SettingRow
          title="Prune ignored files on archive"
          control={
            <Toggle
              aria-label="Prune ignored files on archive"
              checked={git.pruneIgnoredOnArchive}
              onChange={(v) => patch({ pruneIgnoredOnArchive: v })}
            />
          }
        >
          Remove git-ignored files when archiving. Files in{" "}
          <code className="text-meta">worktree.copy</code> (e.g.{" "}
          <code className="text-meta">.env</code>) are kept.
        </SettingRow>
      </div>
    </Card>
  );
}
