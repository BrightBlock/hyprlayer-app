import { useEffect, useState } from "react";

import type { CreateProjectOutcome, Project } from "@/lib/bindings";
import { events } from "@/lib/bindings";
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
import { PathField } from "@/components/PathField";
import { Icon } from "@/components/Icon";
import { RepoAvatar } from "@/components/RepoAvatar";
import { githubRepoRef } from "@/lib/github";
import { basename } from "@/lib/path";
import {
  useBootstrap,
  useCloneProject,
  useCreateProject,
  useCreateThoughtsProfile,
  useDefaultUsername,
  useImportLocalFolder,
  useGithubOwners,
  useImportProject,
  usePublishProjectToGithub,
  useRepoPathInfo,
  useThoughtsProfiles,
} from "@/lib/queries";
import { useDebounced } from "@/lib/useDebounced";
import {
  CreateProjectFields,
  githubRepoName,
  isReservedRepoName,
  sanitizeProjectName,
  type CreateSpec,
} from "./CreateProjectFields";
import { CreateProgressPanel, applyProgress, type ProgressMap } from "./CreateProgressPanel";
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
  const [spec, setSpec] = useState<CreateSpec>({
    name: "",
    location: "",
    branch: "main",
    readme: true,
    gitignore: null,
    github: true,
    owner: null,
    private: true,
  });
  const [progress, setProgress] = useState<ProgressMap>({});
  const [stalled, setStalled] = useState<CreateProjectOutcome | null>(null);
  // Seeded from the spec when a publish stalls, so a name or owner GitHub
  // rejected can be corrected before retrying rather than resent unchanged.
  const [retry, setRetry] = useState<{ owner: string | null; name: string }>({
    owner: null,
    name: "",
  });
  const [opId, setOpId] = useState<string | null>(null);

  const [profileName, setProfileName] = useState<string | null>(null);
  const [folder, setFolder] = useState(AUTO_FOLDER);
  const [customFolder, setCustomFolder] = useState("");
  const [newProfileOpen, setNewProfileOpen] = useState(false);
  const [npName, setNpName] = useState("");
  const [npRepo, setNpRepo] = useState("");

  const clone = useCloneProject();
  const importProject = useImportProject();
  const importFolder = useImportLocalFolder();
  const createProject = useCreateProject();
  const publishToGithub = usePublishProjectToGithub();
  const owners = useGithubOwners();
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

  const isCreate = source.mode === "create";
  const pending =
    clone.isPending || importProject.isPending || importFolder.isPending || createProject.isPending;
  const busy = pending || publishToGithub.isPending;
  const error = clone.error ?? importProject.error ?? importFolder.error ?? createProject.error;
  const ownerList = owners.data ?? [];
  const ownersState = {
    owners: ownerList,
    loading: owners.isLoading,
    unreachable: owners.isError,
  };
  const effectiveOwner = spec.owner ?? ownerList[0]?.login ?? null;
  const remoteName = githubRepoName(spec.name);
  const githubTarget =
    isCreate && spec.github && effectiveOwner && !isReservedRepoName(remoteName)
      ? { owner: effectiveOwner, name: remoteName, private: spec.private }
      : null;
  const retryName = githubRepoName(retry.name);
  const retryTarget =
    retry.owner && !isReservedRepoName(retryName)
      ? { owner: retry.owner, name: retryName, private: spec.private }
      : null;
  const busyPanel = createProject.isPending || stalled !== null;

  useEffect(() => {
    if (configMissing && defaultUser.data) {
      setUser((cur) => cur || defaultUser.data);
    }
  }, [configMissing, defaultUser.data]);

  useEffect(() => {
    const apply = (root: string) => {
      setSource((cur) => (cur.mode === "clone" && !cur.location ? { ...cur, location: root } : cur));
      setSpec((cur) => (cur.location ? cur : { ...cur, location: root }));
    };
    // The server answers this, because only it can: it resolves the configured
    // `repos_root` or its own default, on the machine the clone actually lands
    // on. This used to be `join(homeDir(), "hyprlayer", "repos")` — *this*
    // machine's home — which against a remote host prefilled a macOS path into
    // a Linux server and failed at its unwritable root with a bare
    // "Permission denied (os error 13)" naming no path.
    const root = bootstrap.data?.defaultReposRoot;
    if (root) apply(root);
  }, [bootstrap.data?.defaultReposRoot]);

  useEffect(() => {
    if (!isCreate || !opId) return;
    let live = true;
    let unlisten: (() => void) | null = null;
    void events.projectCreateProgress
      .listen((e) => {
        if (e.payload.opId !== opId) return;
        setProgress((cur) => applyProgress(cur, e.payload));
      })
      .then((off) => {
        if (live) unlisten = off;
        else off();
      })
      .catch(() => {
      });
    return () => {
      live = false;
      unlisten?.();
    };
  }, [isCreate, opId]);

  const profileList = profiles.data ?? [];
  const selectedProfile = profileList.find((p) => p.name === profileName) ?? profileList[0];
  const folders = selectedProfile?.folders ?? [];
  const createName = sanitizeProjectName(spec.name);
  const repoName = isCreate
    ? createName.replace(/[^A-Za-z0-9_-]/g, "_")
    : autoFolderName(source);
  const displayLabel = isCreate ? createName || "project" : repoLabel(source);
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
    if (busy) return;
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
    createProject.reset();
    publishToGithub.reset();
    setProgress({});
    setStalled(null);
    setOpId(null);
  };

  const bootstrapUser = configMissing ? user.trim() || null : null;

  const primaryInput =
    source.mode === "clone"
      ? source.url.trim()
      : source.mode === "disk"
        ? source.path.trim()
        : "";
  const step1Ok = isCreate
    ? Boolean(createName) &&
      (!configMissing || Boolean(user.trim())) &&
      !(spec.github && owners.isLoading)
    : Boolean(primaryInput) &&
      (source.mode !== "disk" || status !== undefined || pathInfo.isError) &&
      (!configMissing || asFolder || Boolean(user.trim())) &&
      (!needsInit || asFolder || Boolean(initBranch.trim())) &&
      !isBare;
  const folderOk = folderChoice === NEW_FOLDER ? Boolean(customFolder.trim()) : true;
  const canSubmit = step1Ok && (asFolder || folderOk) && !newProfileOpen && !pending;

  const [submitIdle, submitBusy] =
    source.mode === "clone"
      ? ["Clone repo", "Cloning…"]
      : isCreate
        ? ["Create project", "Creating…"]
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
    if (isCreate) {
      const id = crypto.randomUUID();
      setProgress({});
      setStalled(null);
      setOpId(id);
      createProject.mutate(
        {
          opId: id,
          name: createName,
          location: spec.location.trim() || null,
          branch: spec.branch.trim() || null,
          scaffold: { readme: spec.readme, gitignore: spec.gitignore },
          github: githubTarget,
          thoughtsDir,
          profile,
          bootstrapUser,
        },
        {
          onSuccess: (outcome) => {
            if (outcome.githubError) {
              setRetry({ owner: githubTarget?.owner ?? effectiveOwner, name: remoteName });
              setStalled(outcome);
            } else {
              succeed(outcome.project);
            }
          },
        },
      );
      return;
    }
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

  const retryGithub = () => {
    if (!stalled || !retryTarget || publishToGithub.isPending) return;
    const id = crypto.randomUUID();
    setOpId(id);
    setProgress((cur) => ({ ...cur, githubRepo: undefined, push: undefined }));
    publishToGithub.mutate(
      {
        opId: id,
        path: stalled.project.path,
        owner: retryTarget.owner,
        name: retryTarget.name,
        isPrivate: retryTarget.private,
      },
      { onSuccess: () => succeed(stalled.project) },
    );
  };

  return (
    <Dialog
      open
      onClose={close}
      title={
        <span className="flex items-baseline justify-between gap-3">
          <span>Add project</span>
          <span className="text-ui font-normal text-ink-dim">
            {stalled
              ? `${displayLabel} · local only`
              : createProject.isPending
                ? `Creating ${displayLabel}`
                : asFolder
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
        {!busyPanel && step === 1 && (
          <>
            <RepoSourcePicker
              value={source}
              onChange={setSource}
              modes={["clone", "disk", "create"]}
              disabled={pending}
              createSlot={
                <CreateProjectFields
                  value={spec}
                  onChange={setSpec}
                  owners={ownersState}
                  effectiveOwner={effectiveOwner}
                  disabled={pending}
                />
              }
              diskLabel={asFolder ? "Folder" : "Repository or folder"}
              diskBrowseTitle="Select a repository or folder"
              diskPlaceholder="/path/to/repo-or-folder"
              diskFooter={
                needsInit ? (
                  <p className="mt-1 text-ui text-ink-dim">
                    {asFolder
                      ? "Added as a plain folder. No git, no thoughts."
                      : "Not a git repository. One will be initialized here."}
                  </p>
                ) : isBare ? (
                  <p className="mt-1 text-ui text-bad">
                    This is a bare repository. Choose a normal working copy instead.
                  </p>
                ) : undefined
              }
            />

            {needsInit && (
              <div className="flex items-center justify-between gap-4">
                <p className="min-w-0 text-body text-ink">Set up a git repository</p>
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
                  className="text-meta"
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
                <p className="mt-1 text-ui text-ink-dim">
                  Attributes your thought documents (one-time, first-run setup).
                </p>
              </Field>
            )}
          </>
        )}

        {!busyPanel && step === 2 && (
          <div className="space-y-4">
            <div className="rounded-md border border-edge bg-panel-2/40 px-3.5 py-3">
              <div className="flex items-center gap-2.5">
                <RepoAvatar
                  source={source.mode === "clone" ? source.url.trim() : null}
                  name={displayLabel}
                  size={24}
                />
                <span className="min-w-0 flex-1 truncate text-body font-medium text-ink">
                  {displayLabel}
                </span>
                {isCreate && <Badge tone="dim">new</Badge>}
              </div>
              {githubTarget && (
                <>
                  <div className="my-1 ml-3 h-3 w-px bg-edge" />
                  <div className="flex items-center gap-2.5">
                    <span className="flex h-6 w-6 items-center justify-center text-ink-dim">
                      <Icon name="cloudUpload" />
                    </span>
                    <span className="min-w-0 flex-1 truncate text-meta text-ink">
                      {githubTarget.owner}/{githubTarget.name}
                    </span>
                    <Badge tone="dim">{githubTarget.private ? "private" : "public"}</Badge>
                  </div>
                </>
              )}
              {previewPath && (
                <>
                  <div className="my-1 ml-3 h-3 w-px bg-edge" />
                  <div className="flex items-center gap-2.5">
                    <HyprlayerMark size={24} />
                    <span className="min-w-0 flex-1 truncate text-body font-medium text-ink">
                      {previewPath}
                    </span>
                    {folderIsNew && <Badge tone="dim">new</Badge>}
                  </div>
                </>
              )}
            </div>

            <div>
              <p className="text-body font-medium text-ink">Thoughts</p>
              <p className="mt-0.5 text-ui text-ink-dim">
                Choose where research, plans & notes for this repo are kept.
              </p>
            </div>

            {profiles.isLoading ? (
              <div className="flex items-center gap-2 py-1 text-ui text-ink-dim">
                <Spinner size="xs" /> Loading profiles…
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
                    <p className="mt-1 text-ui text-warn-ink">
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
                      <PathField
                        value={npRepo}
                        onChange={setNpRepo}
                        placeholder={`~/${npName.trim() || "work"}-thoughts`}
                        disabled={createProfile.isPending}
                        browseTitle="Choose the thoughts repository"
                      />
                    </Field>
                    <p className="text-meta text-ink-dim/80">
                      repos dir: repos · global dir: global
                    </p>
                    {createProfile.error != null && (
                      <ErrorNote error={createProfile.error} onDismiss={() => createProfile.reset()} />
                    )}
                    <div className="flex justify-end gap-2">
                      <Button
                        type="button"
                        className="px-2 py-1 text-ui"
                        onClick={cancelNewProfile}
                        disabled={createProfile.isPending}
                      >
                        Cancel
                      </Button>
                      <Button
                        type="button"
                        variant="primary"
                        className="px-2 py-1 text-ui"
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

        {!busyPanel && error != null && (
          <ErrorNote
            error={error}
            onDismiss={() => {
              clone.reset();
              importProject.reset();
              importFolder.reset();
              createProject.reset();
            }}
          />
        )}

        {busyPanel && (
          <div className="space-y-4">
            <CreateProgressPanel
              progress={progress}
              withGithub={githubTarget !== null || stalled !== null}
            />
            {stalled ? (
              <>
                <ErrorNote error={stalled.githubError} />
                {publishToGithub.error != null && (
                  <ErrorNote
                    error={publishToGithub.error}
                    onDismiss={() => publishToGithub.reset()}
                  />
                )}
                <p className="text-ui leading-prose text-ink-dim">
                  The project is added and usable — only the GitHub step is outstanding.
                </p>
                <div className="flex items-end gap-2">
                  <div className="min-w-0 flex-1">
                    <Field label="Owner">
                      <DropdownSelect
                        aria-label="Retry owner"
                        value={retry.owner ?? ""}
                        onChange={(v) => setRetry((cur) => ({ ...cur, owner: v }))}
                        disabled={publishToGithub.isPending}
                        options={ownerList.map((o) => ({
                          value: o.login,
                          label: o.login,
                          group: o.kind === "user" ? "Your account" : "Organizations",
                        }))}
                      />
                    </Field>
                  </div>
                  <div className="min-w-0 flex-1">
                    <Field label="Repository name">
                      <TextInput
                        value={retry.name}
                        onChange={(e) => setRetry((cur) => ({ ...cur, name: e.target.value }))}
                        spellCheck={false}
                        disabled={publishToGithub.isPending}
                        className="text-meta"
                      />
                    </Field>
                  </div>
                </div>
                <div className="flex justify-end gap-2 pt-1">
                  <Button
                    type="button"
                    disabled={publishToGithub.isPending}
                    onClick={() => succeed(stalled.project)}
                  >
                    Open project
                  </Button>
                  <Button
                    type="button"
                    variant="primary"
                    busy={publishToGithub.isPending}
                    disabled={retryTarget === null}
                    onClick={retryGithub}
                  >
                    Try GitHub again
                  </Button>
                </div>
              </>
            ) : (
              <>
                <p className="text-ui leading-prose text-ink-dim">
                  Everything up to this point is already on disk. Closing now leaves the repository
                  where it is.
                </p>
                <div className="flex justify-end gap-2 pt-1">
                  <Button type="button" variant="primary" busy disabled>
                    {submitBusy}
                  </Button>
                </div>
              </>
            )}
          </div>
        )}

        {!busyPanel && <div className="flex justify-end gap-2 pt-1">
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
        </div>}
      </form>
    </Dialog>
  );
}
