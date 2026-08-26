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
  Toggle,
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
  useImportLocalFolder,
  useImportProject,
  useRepoPathInfo,
  useThoughtsProfiles,
} from "@/lib/queries";
import { useDebounced } from "@/lib/useDebounced";
import { RepoSourcePicker, type RepoSource } from "./RepoSourcePicker";

type Props = {
  open: boolean;
  onClose: () => void;
  initialUrl?: string | null;
  onAdded: (project: Project) => void;
};

const AUTO_FOLDER = "__auto";
const NEW_FOLDER = "__custom";
const DEFAULT_PROFILE = "__default";
const NEW_PROFILE = "__new";

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

function repoLabel(source: RepoSource): string {
  if (source.mode === "clone") {
    const url = source.url.trim();
    const ref = githubRepoRef(url);
    if (ref) return `${ref.owner}/${ref.name}`;
    return basename(url.replace(/\.git$/, "").replace(/[/\\]+$/, "")) || "repository";
  }
  return basename(source.path.trim().replace(/[/\\]+$/, "")) || "repository";
}

export function AddProjectDialog(props: Props) {
  return props.open ? <AddProjectWizard {...props} /> : null;
}

function AddProjectWizard({ onClose, initialUrl, onAdded }: Props) {
  const bootstrap = useBootstrap();
  const configMissing = bootstrap.data?.config.state === "missing";

  const [step, setStep] = useState<1 | 2>(1);
  const [source, setSource] = useState<RepoSource>({
    mode: "clone",
    url: initialUrl ?? "",
    location: "",
  });
  const [user, setUser] = useState("");
  const [initBranch, setInitBranch] = useState("main");
  const [setUpGit, setSetUpGit] = useState(false);

  const [profileName, setProfileName] = useState<string | null>(null);
  const [folder, setFolder] = useState(AUTO_FOLDER);
  const [customFolder, setCustomFolder] = useState("");
  const [newProfileOpen, setNewProfileOpen] = useState(false);
  const [npName, setNpName] = useState("");
  const [npRepo, setNpRepo] = useState("");

  const clone = useCloneProject();
  const importProject = useImportProject();
  const importFolder = useImportLocalFolder();
  const defaultUser = useDefaultUsername();
  const profiles = useThoughtsProfiles();
  const createProfile = useCreateThoughtsProfile();

  const diskPath = source.mode === "disk" ? source.path.trim() : "";
  const debouncedPath = useDebounced(diskPath, 300);
  const pathInfo = useRepoPathInfo(debouncedPath, source.mode === "disk");
  const status = source.mode === "disk" ? pathInfo.data : undefined;
  const needsInit = status === "nonGit";
  const isBare = status === "bare";
  const asFolder = needsInit && !setUpGit;

  const pending = clone.isPending || importProject.isPending || importFolder.isPending;
  const error = clone.error ?? importProject.error ?? importFolder.error;

  useEffect(() => {
    if (configMissing && defaultUser.data) {
      setUser((cur) => cur || defaultUser.data);
    }
  }, [configMissing, defaultUser.data]);

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
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [bootstrap.data?.settings.reposRoot]);

  const profileList = profiles.data ?? [];
  const selectedProfile = profileList.find((p) => p.name === profileName) ?? profileList[0];
  const folders = selectedProfile?.folders ?? [];
  const repoName = autoFolderName(source);
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
    clone.reset();
    importProject.reset();
    importFolder.reset();
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
    (source.mode !== "disk" || status !== undefined || pathInfo.isError) &&
    (!configMissing || asFolder || Boolean(user.trim())) &&
    (!needsInit || asFolder || Boolean(initBranch.trim())) &&
    !isBare;
  const folderOk = folderChoice === NEW_FOLDER ? Boolean(customFolder.trim()) : true;
  const canSubmit = step1Ok && (asFolder || folderOk) && !newProfileOpen && !pending;

  const [submitIdle, submitBusy] =
    source.mode === "clone"
      ? ["Clone repo", "Cloning…"]
      : asFolder
        ? ["Add folder", "Adding…"]
        : needsInit
          ? ["Initialize & add", "Initializing…"]
          : ["Add project", "Importing…"];

  const submit = () => {
    if (!canSubmit) return;
    if (asFolder) {
      importFolder.mutate(
        { path: primaryInput, directoryName: null },
        { onSuccess: succeed },
      );
      return;
    }
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
    }
  };

  return (
    <Dialog
      open
      onClose={close}
      title={
        <span className="flex items-baseline justify-between gap-3">
          <span>Add project</span>
          <span className="text-xs font-normal tracking-normal text-ink-dim">
            {asFolder
              ? "Local folder"
              : step === 1
                ? "Step 1 of 2 · Repository"
                : "Step 2 of 2 · Thoughts"}
          </span>
        </span>
      }
    >
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (step === 1 && !asFolder) {
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
              diskLabel={asFolder ? "Folder" : "Repository or folder"}
              diskBrowseTitle="Select a repository or folder"
              diskPlaceholder="/path/to/repo-or-folder"
              diskFooter={
                needsInit ? (
                  <p className="mt-1 text-xs text-ink-dim">
                    {asFolder
                      ? "Added as a plain folder. No git, no thoughts."
                      : "Not a git repository. One will be initialized here."}
                  </p>
                ) : isBare ? (
                  <p className="mt-1 text-xs text-bad">
                    This is a bare repository. Choose a normal working copy instead.
                  </p>
                ) : undefined
              }
            />

            {needsInit && (
              <div className="flex items-center justify-between gap-4">
                <p className="min-w-0 text-sm text-ink">Set up a git repository</p>
                <Toggle
                  aria-label="Set up a git repository"
                  checked={setUpGit}
                  onChange={setSetUpGit}
                  disabled={pending}
                />
              </div>
            )}

            {needsInit && !asFolder && (
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

            {configMissing && !asFolder && (
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
                      Couldn't read existing folders: {selectedProfile.foldersError}
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

        {error != null && (
          <ErrorNote
            error={error}
            onDismiss={() => {
              clone.reset();
              importProject.reset();
              importFolder.reset();
            }}
          />
        )}

        <div className="flex justify-end gap-2 pt-1">
          {step === 1 ? (
            <>
              <Button type="button" onClick={close} disabled={pending}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                busy={asFolder && pending}
                disabled={asFolder ? !canSubmit : !step1Ok}
              >
                {asFolder ? (pending ? submitBusy : submitIdle) : "Next"}
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
