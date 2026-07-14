import { useEffect, useState } from "react";
import { homeDir, join } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";

import type { Project } from "@/lib/bindings";
import {
  Badge,
  Button,
  Dialog,
  DropdownSelect,
  ErrorNote,
  Field,
  Spinner,
  TextInput,
} from "@/components/ui";
import { HyprlayerMark } from "@/components/Logo";
import { RepoAvatar } from "@/components/RepoAvatar";
import { githubRepoRef } from "@/lib/github";
import { basename } from "@/lib/path";
import {
  useBootstrap,
  useCloneProject,
  useCreateThoughtsProfile,
  useDefaultUsername,
  useImportProject,
  useRepoPathInfo,
  useThoughtsProfiles,
} from "@/lib/queries";
import { useDebounced } from "@/lib/useDebounced";
import { RepoSourcePicker, type RepoSource } from "./RepoSourcePicker";

type Props = {
  open: boolean;
  onClose: () => void;
  /** Seed the clone URL when opening — how a repo picked in Discover enters
   *  this wizard instead of being cloned directly. */
  initialUrl?: string | null;
  /** The freshly added project, so the caller can navigate to it. */
  onAdded: (project: Project) => void;
};

/** Sentinel option values for the Thoughts-step selects. Real values never
 *  collide: the backend reserves `__`-prefixed profile names, and the folder
 *  options below filter `__`-prefixed directory names out. */
const AUTO_FOLDER = "__auto";
const NEW_FOLDER = "__custom";
const DEFAULT_PROFILE = "__default";
const NEW_PROFILE = "__new";

/** Mirrors the CLI's `--directory` sanitization for the auto-folder preview:
 *  last path segment, `.git` stripped, unsafe chars → `_`
 *  (`brightblock.ai` → `brightblock_ai`). Advisory — the CLI recomputes. */
function autoFolderName(source: RepoSource): string {
  const raw = source.mode === "clone" ? source.url : source.path;
  const seg = basename(
    raw
      .trim()
      .replace(/\.git$/, "")
      .replace(/[/\\]+$/, ""),
  );
  return seg.replace(/[^A-Za-z0-9_-]/g, "_");
}

/** `owner/repo` for a recognizable GitHub URL, else the last path segment. */
function repoLabel(source: RepoSource): string {
  if (source.mode === "clone") {
    const url = source.url.trim();
    const ref = githubRepoRef(url);
    if (ref) return `${ref.owner}/${ref.name}`;
    return basename(url.replace(/\.git$/, "").replace(/[/\\]+$/, "")) || "repository";
  }
  return basename(source.path.trim().replace(/[/\\]+$/, "")) || "repository";
}

/**
 * Add a project in two steps. Step 1 — the repository: clone a hosted repo
 * into the repos root, or import a local repo from disk (registered in place,
 * never copied, so worktrees branch off the user's existing repo). Step 2 —
 * thoughts: pick the profile and folder the repo's research/plans/notes map
 * into, with a live preview of the resulting path.
 */
export function AddProjectDialog(props: Props) {
  // Unmount when closed: every piece of wizard state resets for free, so
  // there is no reset() ledger to keep in lockstep with new fields, and
  // `initialUrl` is plain initial state rather than a synchronized effect.
  return props.open ? <AddProjectWizard {...props} /> : null;
}

function AddProjectWizard({ onClose, initialUrl, onAdded }: Props) {
  const bootstrap = useBootstrap();
  // A true first run has no config yet; the CLI needs a username to bootstrap one.
  const configMissing = bootstrap.data?.config.state === "missing";

  const [step, setStep] = useState<1 | 2>(1);
  const [source, setSource] = useState<RepoSource>({
    mode: "clone",
    url: initialUrl ?? "",
    location: "",
  });
  const [user, setUser] = useState("");
  const [initBranch, setInitBranch] = useState("main");

  // Step 2 — where the repo maps into thoughts.
  const [profileName, setProfileName] = useState<string | null>(null);
  const [folder, setFolder] = useState(AUTO_FOLDER);
  const [customFolder, setCustomFolder] = useState("");
  const [newProfileOpen, setNewProfileOpen] = useState(false);
  const [npName, setNpName] = useState("");
  const [npRepo, setNpRepo] = useState("");

  const clone = useCloneProject();
  const importProject = useImportProject();
  const defaultUser = useDefaultUsername();
  // Prefetch on mount so the Thoughts step renders instantly after Next.
  const profiles = useThoughtsProfiles();
  const createProfile = useCreateThoughtsProfile();

  // Probe the browsed disk path (debounced) to tailor the offer before submit;
  // advisory only, since `importProject` re-classifies on the backend anyway.
  const diskPath = source.mode === "disk" ? source.path.trim() : "";
  const debouncedPath = useDebounced(diskPath, 300);
  const pathInfo = useRepoPathInfo(debouncedPath, source.mode === "disk");
  const status = source.mode === "disk" ? pathInfo.data : undefined;
  const needsInit = status === "nonGit";
  const isBare = status === "bare";

  const pending = clone.isPending || importProject.isPending;
  const error = clone.error ?? importProject.error;

  // First run: prefill "Your name" with the shell username ($USER), matching
  // the CLI default, without clobbering an in-session edit.
  useEffect(() => {
    if (configMissing && defaultUser.data) {
      setUser((cur) => cur || defaultUser.data);
    }
  }, [configMissing, defaultUser.data]);

  // Default the clone location to the configured repos root, else ~/hyprlayer/repos.
  useEffect(() => {
    const configured = bootstrap.data?.settings.reposRoot;
    if (configured) {
      setSource((cur) =>
        cur.mode === "clone" && !cur.location ? { ...cur, location: configured } : cur,
      );
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const def = await join(await homeDir(), "hyprlayer", "repos");
        if (!cancelled) {
          setSource((cur) =>
            cur.mode === "clone" && !cur.location ? { ...cur, location: def } : cur,
          );
        }
      } catch {
        // Leave blank → the backend falls back to its own default.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [bootstrap.data?.settings.reposRoot]);

  // ── Step 2 derivations ────────────────────────────────────────────────────
  const profileList = profiles.data ?? [];
  const selectedProfile = profileList.find((p) => p.name === profileName) ?? profileList[0];
  const folders = selectedProfile?.folders ?? [];
  const repoName = autoFolderName(source);
  // A stale pick (a folder from another repo/profile) self-heals to Auto —
  // derived, so no reset bookkeeping when the URL or profile changes.
  const folderChoice =
    folder === AUTO_FOLDER || folder === NEW_FOLDER || folders.includes(folder)
      ? folder
      : AUTO_FOLDER;
  const autoExists = repoName !== "" && folders.includes(repoName);
  const resolvedFolder =
    folderChoice === AUTO_FOLDER
      ? repoName
      : folderChoice === NEW_FOLDER
        ? customFolder.trim()
        : folderChoice;
  const folderIsNew = folderChoice === NEW_FOLDER || (folderChoice === AUTO_FOLDER && !autoExists);
  const previewPath =
    selectedProfile?.thoughtsRepo && selectedProfile.reposDir
      ? `${selectedProfile.thoughtsRepo}/${selectedProfile.reposDir}/${resolvedFolder || "<repo-name>"}/`
      : null;

  const close = () => {
    if (pending) return;
    onClose();
  };

  const succeed = (project: Project) => {
    onClose();
    onAdded(project);
  };

  const goBack = () => {
    setStep(1);
    setNewProfileOpen(false);
    createProfile.reset();
  };

  const bootstrapUser = configMissing ? user.trim() || null : null;

  const primaryInput =
    source.mode === "clone"
      ? source.url.trim()
      : source.mode === "disk"
        ? source.path.trim()
        : "";
  const step1Ok =
    Boolean(primaryInput) &&
    (!configMissing || Boolean(user.trim())) &&
    (!needsInit || Boolean(initBranch.trim())) &&
    !isBare;
  const folderOk = folderChoice === NEW_FOLDER ? Boolean(customFolder.trim()) : true;
  const canSubmit = step1Ok && folderOk && !newProfileOpen && !pending;

  const [submitIdle, submitBusy] =
    source.mode === "clone"
      ? ["Clone repo", "Cloning…"]
      : needsInit
        ? ["Initialize & add", "Initializing…"]
        : ["Add project", "Importing…"];

  const submit = () => {
    if (!canSubmit) return;
    // Auto = let the CLI derive the folder; anything else is an explicit pick.
    const thoughtsDir = folderChoice === AUTO_FOLDER ? null : resolvedFolder || null;
    const profile = selectedProfile?.name ?? null;
    if (source.mode === "clone") {
      clone.mutate(
        {
          url: primaryInput,
          location: source.location.trim() || null,
          name: null,
          thoughtsDir,
          profile,
          bootstrapUser,
        },
        { onSuccess: succeed },
      );
    } else if (source.mode === "disk") {
      // `branch` only matters when we're creating the repo; the backend
      // re-classifies anyway, so this stays correct even if `needsInit` is stale.
      importProject.mutate(
        {
          path: primaryInput,
          branch: needsInit ? initBranch.trim() || "main" : null,
          directoryName: null,
          thoughtsDir,
          profile,
          bootstrapUser,
        },
        { onSuccess: succeed },
      );
    }
  };

  const openNewProfile = () => {
    setNewProfileOpen(true);
    setNpName("");
    setNpRepo("");
    createProfile.reset();
  };

  const cancelNewProfile = () => {
    setNewProfileOpen(false);
    createProfile.reset();
  };

  const submitNewProfile = () => {
    const name = npName.trim();
    if (!name || createProfile.isPending) return;
    createProfile.mutate(
      { name, thoughtsRepo: npRepo.trim() || null },
      {
        onSuccess: (created) => {
          setProfileName(created.name);
          setNewProfileOpen(false);
        },
      },
    );
  };

  const browseNpRepo = async () => {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Choose the thoughts repository",
      });
      if (typeof picked === "string") setNpRepo(picked);
    } catch {
      // Native picker unavailable or dismissed — the field stays hand-editable.
    }
  };

  return (
    <Dialog
      open
      onClose={close}
      className="max-w-md"
      title={
        <span className="flex items-baseline justify-between gap-3">
          <span>Add project</span>
          <span className="text-xs font-normal tracking-normal text-ink-dim">
            {step === 1 ? "Step 1 of 2 · Repository" : "Step 2 of 2 · Thoughts"}
          </span>
        </span>
      }
    >
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (step === 1) {
            if (step1Ok) setStep(2);
          } else {
            submit();
          }
        }}
      >
        {step === 1 && (
          <>
            <RepoSourcePicker
              value={source}
              onChange={setSource}
              modes={["clone", "disk"]}
              disabled={pending}
              diskFooter={
                needsInit ? (
                  <p className="mt-1 text-xs text-ink-dim">
                    Not a git repository — hyprlayer can set one up for you.
                  </p>
                ) : isBare ? (
                  <p className="mt-1 text-xs text-bad">
                    This is a bare repository — choose a normal working copy instead.
                  </p>
                ) : undefined
              }
            />

            {needsInit && (
              <Field label="Initial branch">
                <TextInput
                  value={initBranch}
                  onChange={(e) => setInitBranch(e.target.value)}
                  placeholder="main"
                  disabled={pending}
                  className="font-mono text-xs"
                />
              </Field>
            )}

            {configMissing && (
              <Field label="Your name">
                <TextInput
                  value={user}
                  onChange={(e) => setUser(e.target.value)}
                  placeholder="e.g. jane"
                  disabled={pending}
                />
                <p className="mt-1 text-xs text-ink-dim">
                  Attributes your thought documents (one-time, first-run setup).
                </p>
              </Field>
            )}
          </>
        )}

        {step === 2 && (
          <div className="space-y-4">
            {/* What's being added, and where its thoughts will land. */}
            <div className="rounded-md border border-edge bg-panel-2/40 px-3.5 py-3">
              <div className="flex items-center gap-2.5">
                <RepoAvatar
                  source={source.mode === "clone" ? source.url.trim() : null}
                  name={repoLabel(source)}
                  size={24}
                />
                <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">
                  {repoLabel(source)}
                </span>
              </div>
              {previewPath && (
                <>
                  <div className="my-1 ml-3 h-3 w-px bg-edge" />
                  <div className="flex items-center gap-2.5">
                    <HyprlayerMark size={24} />
                    <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">
                      {previewPath}
                    </span>
                    {folderIsNew && <Badge tone="dim">new</Badge>}
                  </div>
                </>
              )}
            </div>

            <div>
              <p className="text-sm font-medium text-ink">Thoughts</p>
              <p className="mt-0.5 text-xs text-ink-dim">
                Choose where research, plans & notes for this repo are kept.
              </p>
            </div>

            {profiles.isLoading ? (
              <div className="flex items-center gap-2 py-1 text-xs text-ink-dim">
                <Spinner size={12} /> Loading profiles…
              </div>
            ) : profiles.error != null ? (
              <ErrorNote error={profiles.error} />
            ) : (
              <>
                <Field label="Profile">
                  <DropdownSelect
                    value={newProfileOpen ? NEW_PROFILE : (selectedProfile?.name ?? DEFAULT_PROFILE)}
                    onChange={(v) => {
                      if (v === NEW_PROFILE) {
                        openNewProfile();
                      } else {
                        setProfileName(v === DEFAULT_PROFILE ? null : v);
                        setNewProfileOpen(false);
                      }
                    }}
                    disabled={pending}
                    options={[
                      ...profileList.map((p) => ({
                        value: p.name ?? DEFAULT_PROFILE,
                        label: p.name ?? "default",
                      })),
                      // Creating a profile needs an existing config to write into.
                      ...(configMissing ? [] : [{ value: NEW_PROFILE, label: "New profile…" }]),
                    ]}
                  />
                </Field>

                <Field label="Folder">
                  <DropdownSelect
                    value={folderChoice}
                    onChange={setFolder}
                    disabled={pending || newProfileOpen}
                    options={[
                      {
                        value: AUTO_FOLDER,
                        label: repoName
                          ? `${repoName} (${autoExists ? "existing" : "new"})`
                          : "Named after repo",
                      },
                      ...folders
                        .filter((f) => f !== repoName && !f.startsWith("__"))
                        .map((f) => ({ value: f, label: f })),
                      { value: NEW_FOLDER, label: "New folder…" },
                    ]}
                  />
                  {selectedProfile?.foldersError && (
                    <p className="mt-1 text-xs text-warn">
                      Couldn't read existing folders — {selectedProfile.foldersError}
                    </p>
                  )}
                </Field>

                {newProfileOpen && (
                  <div className="flex flex-col gap-3 rounded-md border border-edge bg-panel-2/50 p-3">
                    <Field label="Profile name">
                      <TextInput
                        value={npName}
                        onChange={(e) => setNpName(e.target.value)}
                        placeholder="work"
                        spellCheck={false}
                        autoFocus
                        disabled={createProfile.isPending}
                      />
                    </Field>
                    <Field label="Thoughts repository path">
                      <div className="flex gap-2">
                        <TextInput
                          value={npRepo}
                          onChange={(e) => setNpRepo(e.target.value)}
                          placeholder={`~/${npName.trim() || "work"}-thoughts`}
                          spellCheck={false}
                          className="font-mono text-xs"
                          disabled={createProfile.isPending}
                        />
                        <Button
                          type="button"
                          onClick={() => void browseNpRepo()}
                          disabled={createProfile.isPending}
                        >
                          Browse
                        </Button>
                      </div>
                    </Field>
                    <p className="font-mono text-xs text-ink-dim/80">
                      repos dir: repos · global dir: global
                    </p>
                    {createProfile.error != null && (
                      <ErrorNote error={createProfile.error} onDismiss={() => createProfile.reset()} />
                    )}
                    <div className="flex justify-end gap-2">
                      <Button
                        type="button"
                        className="px-2 py-1 text-xs"
                        onClick={cancelNewProfile}
                        disabled={createProfile.isPending}
                      >
                        Cancel
                      </Button>
                      <Button
                        type="button"
                        variant="primary"
                        className="px-2 py-1 text-xs"
                        busy={createProfile.isPending}
                        disabled={!npName.trim()}
                        onClick={submitNewProfile}
                      >
                        Create profile
                      </Button>
                    </div>
                  </div>
                )}

                {folderChoice === NEW_FOLDER && !newProfileOpen && (
                  <Field label="New folder name">
                    <TextInput
                      value={customFolder}
                      onChange={(e) => setCustomFolder(e.target.value)}
                      placeholder={repoName || "my-project"}
                      spellCheck={false}
                      autoFocus
                      disabled={pending}
                    />
                  </Field>
                )}
              </>
            )}
          </div>
        )}

        {error != null && <ErrorNote error={error} />}

        <div className="flex justify-end gap-2 pt-1">
          {step === 1 ? (
            <>
              <Button type="button" onClick={close} disabled={pending}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" disabled={!step1Ok}>
                Next
              </Button>
            </>
          ) : (
            <>
              <Button type="button" onClick={goBack} disabled={pending}>
                Back
              </Button>
              <Button type="submit" variant="primary" busy={pending} disabled={!canSubmit}>
                {pending ? submitBusy : submitIdle}
              </Button>
            </>
          )}
        </div>
      </form>
    </Dialog>
  );
}
