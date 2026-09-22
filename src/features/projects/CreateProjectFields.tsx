import { type ReactNode } from "react";

import type { GithubOwner, GitignoreTemplate } from "@/lib/bindings";
import { DropdownSelect, Field, Spinner, TextInput, Toggle, cx } from "@/components/ui";
import { PathField } from "@/components/PathField";
import { CheckIcon, GlobeIcon, LockIcon, UploadCloudIcon } from "@/components/icons";

export type CreateSpec = {
  name: string;
  location: string;
  branch: string;
  readme: boolean;
  gitignore: GitignoreTemplate | null;
  github: boolean;
  owner: string | null;
  private: boolean;
};

export const GITIGNORE_TEMPLATES: Array<{ value: GitignoreTemplate; label: string }> = [
  { value: "node", label: "Node" },
  { value: "rust", label: "Rust" },
  { value: "python", label: "Python" },
  { value: "go", label: "Go" },
];

export function sanitizeProjectName(raw: string): string {
  return raw.trim().replace(/[/\\]/g, "-");
}

// Mirrors `repos::github_repo_name` in hyprlayer-core, which is the authority —
// this copy only drives the preview, so the user sees the name GitHub will
// actually create rather than discovering the rewrite afterwards.
export function githubRepoName(raw: string): string {
  return raw
    .trim()
    .replace(/[^A-Za-z0-9._-]/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-+|-+$/g, "");
}

const RESERVED_REPO_NAMES = ["", ".", "..", ".git"];

export function isReservedRepoName(name: string): boolean {
  return RESERVED_REPO_NAMES.includes(name);
}

// The owner list is owned by the dialog, not by this component: these fields
// unmount when the wizard advances, and an owner resolved by an effect in here
// would be lost for any create submitted before the query settled.
export type OwnersState = {
  owners: GithubOwner[];
  loading: boolean;
  unreachable: boolean;
};

type Props = {
  value: CreateSpec;
  onChange: (next: CreateSpec) => void;
  owners: OwnersState;
  effectiveOwner: string | null;
  disabled?: boolean;
};

export function CreateProjectFields({
  value,
  onChange,
  owners,
  effectiveOwner,
  disabled = false,
}: Props) {
  const ownerList = owners.owners;
  const signedIn = ownerList.length > 0;
  const set = (patch: Partial<CreateSpec>) => onChange({ ...value, ...patch });

  const name = sanitizeProjectName(value.name);
  const remoteName = githubRepoName(value.name);
  const remoteNameOk = !isReservedRepoName(remoteName);
  const owner = effectiveOwner;

  return (
    <>
      <Field label="Project name">
        <TextInput
          value={value.name}
          onChange={(e) => set({ name: e.target.value })}
          placeholder="my-project"
          spellCheck={false}
          autoFocus
          disabled={disabled}
        />
        <p className="mt-1 font-mono text-xs text-ink-dim">
          {value.location.trim().replace(/[/\\]+$/, "") || "~"}/
          <span className="text-ink">{name || "<name>"}</span>
        </p>
      </Field>

      <Field label="Location">
        <PathField
          value={value.location}
          onChange={(location) => set({ location })}
          placeholder="~/hyprlayer/repos"
          disabled={disabled}
          browseTitle="Choose where to create the project"
        />
      </Field>

      <Field label="Initial branch">
        <TextInput
          value={value.branch}
          onChange={(e) => set({ branch: e.target.value })}
          placeholder="main"
          disabled={disabled}
          className="font-mono text-xs"
        />
      </Field>

      <div>
        <span className="mb-1 block text-xs font-medium text-ink-dim">Start with</span>
        <div className="flex flex-wrap items-center gap-1.5">
          <ScaffoldChip
            label="README.md"
            checked={value.readme}
            disabled={disabled}
            onClick={() => set({ readme: !value.readme })}
          />
          <ScaffoldChip
            label=".gitignore"
            checked={value.gitignore !== null}
            disabled={disabled}
            onClick={() => set({ gitignore: value.gitignore === null ? "node" : null })}
          />
          {value.gitignore !== null && (
            <DropdownSelect
              aria-label="Gitignore template"
              value={value.gitignore}
              onChange={(v) => set({ gitignore: v as GitignoreTemplate })}
              disabled={disabled}
              options={GITIGNORE_TEMPLATES}
              className="w-auto px-2.5 py-1 text-xs"
            />
          )}
        </div>
        {!value.readme && value.gitignore === null && (
          <p className="mt-1.5 text-xs text-ink-dim">
            An empty first commit is made either way, so{" "}
            <span className="font-mono text-ink">{value.branch.trim() || "main"}</span> exists and
            can be pushed.
          </p>
        )}
      </div>

      <div className="rounded-md border border-edge bg-panel-2/40 px-3.5 py-3">
        <div className="flex items-center gap-2.5">
          <UploadCloudIcon size={16} className="shrink-0 text-ink-dim" />
          <p className={cx("min-w-0 flex-1 text-sm", signedIn ? "text-ink" : "text-ink-dim")}>
            Create on GitHub
          </p>
          {owners.loading ? (
            <Spinner size={14} />
          ) : (
            <Toggle
              aria-label="Create on GitHub"
              checked={value.github && signedIn}
              onChange={(next) => set({ github: next })}
              disabled={disabled || !signedIn}
            />
          )}
        </div>

        {!signedIn && !owners.loading && (
          <p className="mt-2 text-xs leading-relaxed text-ink-dim">
            {owners.unreachable ? (
              <>
                GitHub could not be reached, so this project will be created locally only. Check
                your connection and reopen this dialog to publish it.
              </>
            ) : (
              <>
                Sign in with <span className="font-mono text-ink">gh auth login</span> to push this
                repo to GitHub. The project is still created locally.
              </>
            )}
          </p>
        )}

        {signedIn && value.github && (
          <>
            <div className="mt-3 flex items-end gap-2">
              <div className="min-w-0 flex-1">
                <Field label="Owner">
                  <DropdownSelect
                    aria-label="Owner"
                    value={owner ?? ""}
                    onChange={(v) => set({ owner: v })}
                    disabled={disabled}
                    options={ownerList.map((o) => ({
                      value: o.login,
                      label: o.login,
                      group: o.kind === "user" ? "Your account" : "Organizations",
                    }))}
                  />
                </Field>
              </div>
              <div className="shrink-0">
                <span className="mb-1 block text-xs font-medium text-ink-dim">Visibility</span>
                <div className="flex gap-0.5 rounded-md border border-edge bg-panel p-0.5">
                  <VisibilityChip
                    label="Private"
                    selected={value.private}
                    disabled={disabled}
                    onClick={() => set({ private: true })}
                    icon={<LockIcon size={12} />}
                  />
                  <VisibilityChip
                    label="Public"
                    selected={!value.private}
                    disabled={disabled}
                    onClick={() => set({ private: false })}
                    icon={<GlobeIcon size={12} />}
                  />
                </div>
              </div>
            </div>
            {remoteNameOk ? (
              <>
                <p className="mt-2.5 font-mono text-xs text-ink-dim">
                  github.com/
                  <span className="text-ink">
                    {owner ?? "owner"}/{remoteName || "<name>"}
                  </span>
                </p>
                {remoteName !== name && name !== "" && (
                  <p className="mt-1 text-xs text-ink-dim">
                    GitHub only allows letters, digits, dots, dashes and underscores, so the
                    repository is named <span className="font-mono text-ink">{remoteName}</span>.
                  </p>
                )}
              </>
            ) : (
              <p className="mt-2.5 text-xs text-warn">
                GitHub can't name a repository after{" "}
                <span className="font-mono">{value.name.trim()}</span>. Use letters, digits, dots,
                dashes or underscores.
              </p>
            )}
          </>
        )}
      </div>
    </>
  );
}

function ScaffoldChip({
  label,
  checked,
  disabled,
  onClick,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={onClick}
      disabled={disabled}
      className={cx(
        "inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 font-mono text-xs font-medium transition-colors",
        "disabled:cursor-not-allowed disabled:opacity-50",
        checked
          ? "border-accent/30 bg-accent/10 text-ink"
          : "border-edge text-ink-dim hover:bg-edge/40",
      )}
    >
      {checked ? (
        <CheckIcon size={11} className="text-accent" />
      ) : (
        <span aria-hidden="true" className="h-[11px] w-[11px] rounded-[3px] border border-edge" />
      )}
      {label}
    </button>
  );
}

function VisibilityChip({
  label,
  selected,
  disabled,
  onClick,
  icon,
}: {
  label: string;
  selected: boolean;
  disabled: boolean;
  onClick: () => void;
  icon: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      disabled={disabled}
      className={cx(
        "inline-flex items-center gap-1.5 rounded border px-2 py-1 text-xs font-medium transition-colors",
        "disabled:cursor-not-allowed disabled:opacity-50",
        selected
          ? "border-edge bg-panel-2 text-ink"
          : "border-transparent text-ink-dim hover:text-ink",
      )}
    >
      {icon}
      {label}
    </button>
  );
}
