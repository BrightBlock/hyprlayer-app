// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { GithubOwner, RepoPathStatus, ThoughtsProfile } from "@/lib/bindings";
import { AddProjectDialog } from "./AddProjectDialog";

const q = vi.hoisted(() => ({
  useBootstrap: vi.fn(),
  useCloneProject: vi.fn(),
  useImportProject: vi.fn(),
  useImportLocalFolder: vi.fn(),
  useDefaultUsername: vi.fn(),
  useRepoPathInfo: vi.fn(),
  useRecentRepos: vi.fn(),
  useThoughtsProfiles: vi.fn(),
  useCreateThoughtsProfile: vi.fn(),
  useCreateProject: vi.fn(),
  usePublishProjectToGithub: vi.fn(),
  useGithubOwners: vi.fn(),
  useConnectionStatus: vi.fn(
    (): { data: { host: { destination: string; label: string } | null } } => ({
      data: { host: null },
    }),
  ),
}));
vi.mock("@/lib/queries", () => q);
vi.mock("@/lib/useDebounced", () => ({ useDebounced: (v: unknown) => v }));
vi.mock("@tauri-apps/api/path", () => ({
  homeDir: () => Promise.resolve("/home"),
  join: (...p: string[]) => Promise.resolve(p.join("/")),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

afterEach(cleanup);

function mutation() {
  return { mutate: vi.fn(), reset: vi.fn(), isPending: false, isError: false, error: null };
}

const defaultProfile: ThoughtsProfile = {
  name: null,
  backendKind: "git",
  thoughtsRepo: "~/thoughts",
  reposDir: "repos",
  globalDir: "global",
  folders: [],
  foldersError: null,
};

function setupHooks(
  profiles: ThoughtsProfile[] = [defaultProfile],
  configState: "ok" | "missing" = "ok",
  owners: GithubOwner[] = [],
) {
  const clone = mutation();
  const importProject = mutation();
  const importFolder = mutation();
  const createProfile = mutation();
  const createProject = mutation();
  const publish = mutation();
  q.useBootstrap.mockReturnValue({
    data: {
      config: { state: configState },
      settings: { reposRoot: "/repos" },
      defaultReposRoot: "/repos",
    },
  });
  q.useCloneProject.mockReturnValue(clone);
  q.useImportProject.mockReturnValue(importProject);
  q.useImportLocalFolder.mockReturnValue(importFolder);
  q.useDefaultUsername.mockReturnValue({ data: "jane" });
  q.useRepoPathInfo.mockReturnValue({ data: undefined });
  q.useRecentRepos.mockReturnValue({ data: [], isLoading: false });
  q.useThoughtsProfiles.mockReturnValue({ data: profiles, isLoading: false, error: null });
  q.useCreateThoughtsProfile.mockReturnValue(createProfile);
  q.useCreateProject.mockReturnValue(createProject);
  q.usePublishProjectToGithub.mockReturnValue(publish);
  q.useGithubOwners.mockReturnValue({ data: owners, isLoading: false, isError: false });
  return { clone, importProject, importFolder, createProfile, createProject, publish };
}

const next = () => fireEvent.click(screen.getByRole("button", { name: "Next" }));

function pick(field: string, option: string) {
  fireEvent.click(screen.getByLabelText(field));
  fireEvent.click(screen.getByRole("option", { name: option }));
  expect(screen.queryByRole("listbox")).toBeNull();
}

function setupDisk(status: RepoPathStatus | undefined, configState: "ok" | "missing" = "ok") {
  const hooks = setupHooks([defaultProfile], configState);
  q.useRepoPathInfo.mockReturnValue({ data: status });

  render(<AddProjectDialog open onClose={vi.fn()} onAdded={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Open from disk" }));
  fireEvent.change(screen.getByPlaceholderText("/path/to/repo-or-folder"), {
    target: { value: "/my/folder" },
  });
  return hooks;
}

function setupClone(profiles: ThoughtsProfile[] = [defaultProfile]) {
  const hooks = setupHooks(profiles);
  render(<AddProjectDialog open onClose={vi.fn()} onAdded={vi.fn()} />);
  fireEvent.change(screen.getByPlaceholderText("https://github.com/user/thoughts.git"), {
    target: { value: "https://github.com/acme/widgets.git" },
  });
  return hooks;
}

describe("AddProjectDialog disk import", () => {
  it("offers a non-git folder as a plain folder by default", () => {
    setupDisk("nonGit");
    expect(screen.getByText(/plain folder/)).toBeDefined();
    expect(
      (screen.getByRole("switch", { name: /Set up a git repository/ }) as HTMLInputElement).checked,
    ).toBe(false);
    expect(screen.getByRole("button", { name: "Add folder" })).toBeDefined();
  });

  it("initializes with the chosen branch when git setup is toggled on", () => {
    const { importProject } = setupDisk("nonGit");
    fireEvent.click(screen.getByRole("switch", { name: /Set up a git repository/ }));
    fireEvent.change(screen.getByPlaceholderText("main"), { target: { value: "master" } });
    next();
    fireEvent.click(screen.getByRole("button", { name: /Initialize & add/ }));
    expect(importProject.mutate).toHaveBeenCalledWith(
      expect.objectContaining({
        path: "/my/folder",
        branch: "master",
        thoughtsDir: null,
        profile: null,
      }),
      expect.anything(),
    );
  });

  it("imports a ready repo in place, with no offer and no branch", () => {
    const { importProject } = setupDisk("ready");
    expect(screen.queryByText(/Not a git repository/)).toBeNull();
    next();
    fireEvent.click(screen.getByRole("button", { name: "Add project" }));
    expect(importProject.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ path: "/my/folder", branch: null }),
      expect.anything(),
    );
  });

  it("refuses a bare repo and blocks at the repository step", () => {
    const { importProject } = setupDisk("bare");
    expect(screen.getByText(/bare repository/i)).toBeDefined();
    const nextButton = screen.getByRole("button", { name: "Next" }) as HTMLButtonElement;
    expect(nextButton.disabled).toBe(true);
    fireEvent.click(nextButton);
    expect(screen.queryByRole("button", { name: "Add project" })).toBeNull();
    expect(importProject.mutate).not.toHaveBeenCalled();
  });

  it("adds a non-git folder as-is, skipping the thoughts step entirely", () => {
    const { importFolder, importProject } = setupDisk("nonGit");
    expect(screen.queryByPlaceholderText("main")).toBeNull();
    expect(screen.queryByText("Repository or folder")).toBeNull();
    expect(screen.getByText("Folder")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
    expect(screen.queryByText("Step 2 of 2 · Thoughts")).toBeNull();
    expect(importProject.mutate).not.toHaveBeenCalled();
    expect(importFolder.mutate).toHaveBeenCalledWith(
      { path: "/my/folder", directoryName: null },
      expect.anything(),
    );
  });

  it("needs no bootstrap username for a folder, it writes no config", () => {
    const { importFolder } = setupDisk("nonGit", "missing");
    expect(screen.queryByLabelText(/Your name/)).toBeNull();
    fireEvent.click(screen.getByRole("switch", { name: /Set up a git repository/ }));
    expect(screen.getByLabelText(/Your name/)).toBeDefined();
    fireEvent.click(screen.getByRole("switch", { name: /Set up a git repository/ }));
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
    expect(importFolder.mutate).toHaveBeenCalled();
  });

  it("drops the folder offer if the path turns out to be a git repo", () => {
    const { importFolder } = setupDisk("nonGit");
    q.useRepoPathInfo.mockReturnValue({ data: "ready" });
    cleanup();
    render(<AddProjectDialog open onClose={vi.fn()} onAdded={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Open from disk" }));
    fireEvent.change(screen.getByPlaceholderText("/path/to/repo-or-folder"), {
      target: { value: "/my/folder" },
    });
    expect(screen.queryByRole("switch", { name: /Set up a git repository/ })).toBeNull();
    next();
    expect(screen.getByText("Step 2 of 2 · Thoughts")).toBeDefined();
    expect(importFolder.mutate).not.toHaveBeenCalled();
  });

  it("waits for the probe before allowing submit (no blind git init)", () => {
    const { importProject } = setupDisk(undefined);
    const nextButton = screen.getByRole("button", { name: "Next" }) as HTMLButtonElement;
    expect(nextButton.disabled).toBe(true);
    fireEvent.click(nextButton);
    expect(screen.queryByRole("button", { name: "Add project" })).toBeNull();
    expect(importProject.mutate).not.toHaveBeenCalled();
  });
});

describe("AddProjectDialog thoughts step", () => {
  it("previews the auto mapping and defaults submit to auto folder + default profile", () => {
    const { clone } = setupClone();
    next();
    expect(screen.getByText("Step 2 of 2 · Thoughts")).toBeDefined();
    expect(screen.getByText("~/thoughts/repos/widgets/")).toBeDefined();
    expect(screen.getByText("new")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(
      (screen.getByPlaceholderText("https://github.com/user/thoughts.git") as HTMLInputElement)
        .value,
    ).toBe("https://github.com/acme/widgets.git");
    next();
    fireEvent.click(screen.getByRole("button", { name: "Clone repo" }));
    expect(clone.mutate).toHaveBeenCalledWith(
      expect.objectContaining({
        url: "https://github.com/acme/widgets.git",
        thoughtsDir: null,
        profile: null,
      }),
      expect.anything(),
    );
  });

  it("submits the picked profile and an existing folder", () => {
    const work: ThoughtsProfile = {
      ...defaultProfile,
      name: "work",
      thoughtsRepo: "~/work-thoughts",
      folders: ["widgets", "platform"],
    };
    const { clone } = setupClone([defaultProfile, work]);
    next();
    pick("Profile", "work");
    expect(screen.getByText("~/work-thoughts/repos/widgets/")).toBeDefined();
    expect(screen.queryByText("new")).toBeNull();
    pick("Folder", "platform");
    fireEvent.click(screen.getByRole("button", { name: "Clone repo" }));
    expect(clone.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ thoughtsDir: "platform", profile: "work" }),
      expect.anything(),
    );
  });

  it("surfaces an unreadable thoughts tree instead of showing it as empty", () => {
    setupClone([{ ...defaultProfile, foldersError: "/Users/x/thoughts/repos: Permission denied" }]);
    next();
    expect(screen.getByText(/Couldn't read existing folders/)).toBeDefined();
    expect(screen.getByText(/Permission denied/)).toBeDefined();
  });

  it("requires a name for a new folder and passes it through", () => {
    const { clone } = setupClone();
    next();
    pick("Folder", "New folder…");
    fireEvent.click(screen.getByRole("button", { name: "Clone repo" }));
    expect(clone.mutate).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("New folder name"), {
      target: { value: "acme-notes" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Clone repo" }));
    expect(clone.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ thoughtsDir: "acme-notes", profile: null }),
      expect.anything(),
    );
  });

  it("seeds the clone URL when opened from a Discover pick", () => {
    const { clone } = setupHooks();
    render(
      <AddProjectDialog
        open
        initialUrl="https://github.com/acme/widgets.git"
        onClose={vi.fn()}
        onAdded={vi.fn()}
      />,
    );
    const url = screen.getByPlaceholderText(
      "https://github.com/user/thoughts.git",
    ) as HTMLInputElement;
    expect(url.value).toBe("https://github.com/acme/widgets.git");
    next();
    expect(screen.getByText("Step 2 of 2 · Thoughts")).toBeDefined();
    expect(screen.getByText("~/thoughts/repos/widgets/")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Clone repo" }));
    expect(clone.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ url: "https://github.com/acme/widgets.git" }),
      expect.anything(),
    );
  });

  it("creates a profile inline and closes the form on success", () => {
    const { createProfile } = setupClone();
    createProfile.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: unknown) => void }) =>
        opts?.onSuccess?.({ ...defaultProfile, name: "team", thoughtsRepo: "~/team-thoughts" }),
    );
    next();
    pick("Profile", "New profile…");
    const submit = screen.getByRole("button", { name: "Clone repo" }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Profile name"), { target: { value: "team" } });
    fireEvent.click(screen.getByRole("button", { name: "Create profile" }));
    expect(createProfile.mutate).toHaveBeenCalledWith(
      { name: "team", thoughtsRepo: null },
      expect.anything(),
    );
    expect(screen.queryByLabelText("Profile name")).toBeNull();
    expect(submit.disabled).toBe(false);
  });
});

describe("AddProjectDialog · create new", () => {
  const owners: GithubOwner[] = [
    { login: "jane", kind: "user" },
    { login: "acme", kind: "organization" },
  ];

  function setupCreate(withOwners: GithubOwner[] = owners) {
    const hooks = setupHooks([defaultProfile], "ok", withOwners);
    render(<AddProjectDialog open onClose={vi.fn()} onAdded={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Create new" }));
    fireEvent.change(screen.getByPlaceholderText("my-project"), {
      target: { value: "aurora-api" },
    });
    return hooks;
  }

  it("creates the repo locally and on GitHub under the chosen org", () => {
    const { createProject } = setupCreate();
    fireEvent.click(screen.getByRole("switch", { name: ".gitignore" }));
    pick("Gitignore template", "Rust");
    pick("Owner", "acme");
    next();
    expect(screen.getByText("acme/aurora-api")).toBeDefined();
    expect(screen.getByText("private")).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Create project" }));
    expect(createProject.mutate).toHaveBeenCalledWith(
      {
        opId: expect.any(String),
        name: "aurora-api",
        location: "/repos",
        branch: "main",
        scaffold: { readme: true, gitignore: "rust" },
        github: { owner: "acme", name: "aurora-api", private: true },
        thoughtsDir: null,
        profile: null,
        bootstrapUser: null,
      },
      expect.anything(),
    );
  });

  it("still creates the project locally when gh has no signed-in account", () => {
    const { createProject } = setupCreate([]);
    expect(screen.getByText(/gh auth login/)).toBeDefined();
    next();
    fireEvent.click(screen.getByRole("button", { name: "Create project" }));
    expect(createProject.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ github: null }),
      expect.anything(),
    );
  });

  it("keeps the project and offers a retry when the GitHub step fails", () => {
    const { createProject, publish } = setupCreate();
    const project = { path: "/repos/aurora-api", mappedName: "aurora-api" };
    createProject.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: unknown) => void }) =>
        opts?.onSuccess?.({
          project,
          githubUrl: null,
          githubError: { code: "gh", message: "Name already exists", remedy: "Pick another name." },
        }),
    );
    next();
    fireEvent.click(screen.getByRole("button", { name: "Create project" }));

    expect(screen.getByText("Name already exists")).toBeDefined();
    expect(screen.getByText("Pick another name.")).toBeDefined();
    expect(screen.getByRole("button", { name: "Open project" })).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Try GitHub again" }));
    expect(publish.mutate).toHaveBeenCalledWith(
      {
        opId: expect.any(String),
        path: "/repos/aurora-api",
        owner: "jane",
        name: "aurora-api",
        isPrivate: true,
      },
      expect.anything(),
    );
  });

  it("retries GitHub under a corrected name after the first one was taken", () => {
    const { createProject, publish } = setupCreate();
    const project = { path: "/repos/aurora-api", mappedName: "aurora-api" };
    createProject.mutate.mockImplementation(
      (_args: unknown, opts?: { onSuccess?: (r: unknown) => void }) =>
        opts?.onSuccess?.({
          project,
          githubUrl: null,
          githubError: { code: "github_name_taken", message: "Name already exists", remedy: null },
        }),
    );
    next();
    fireEvent.click(screen.getByRole("button", { name: "Create project" }));

    fireEvent.change(screen.getByLabelText("Repository name"), {
      target: { value: "aurora-api-2" },
    });
    pick("Retry owner", "acme");
    fireEvent.click(screen.getByRole("button", { name: "Try GitHub again" }));
    expect(publish.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ owner: "acme", name: "aurora-api-2" }),
      expect.anything(),
    );
  });

  it("will not advance past the owner list while it is still loading", () => {
    setupHooks([defaultProfile], "ok", []);
    q.useGithubOwners.mockReturnValue({ data: undefined, isLoading: true, isError: false });
    render(<AddProjectDialog open onClose={vi.fn()} onAdded={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Create new" }));
    fireEvent.change(screen.getByPlaceholderText("my-project"), {
      target: { value: "aurora-api" },
    });
    expect((screen.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("says GitHub is unreachable rather than claiming the user is signed out", () => {
    setupHooks([defaultProfile], "ok", []);
    q.useGithubOwners.mockReturnValue({ data: undefined, isLoading: false, isError: true });
    render(<AddProjectDialog open onClose={vi.fn()} onAdded={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Create new" }));
    expect(screen.getByText(/could not be reached/)).toBeDefined();
    expect(screen.queryByText(/gh auth login/)).toBeNull();
  });

  it("names the thoughts folder after the project", () => {
    setupCreate();
    next();
    expect(screen.getByText("~/thoughts/repos/aurora-api/")).toBeDefined();
  });
  it("names the remote the way GitHub will, and says so", () => {
    const { createProject } = setupCreate();
    fireEvent.change(screen.getByPlaceholderText("my-project"), {
      target: { value: "Aurora API!" },
    });
    expect(screen.getByText("jane/Aurora-API")).toBeDefined();
    expect(screen.getByText(/GitHub only allows/)).toBeDefined();
    next();
    fireEvent.click(screen.getByRole("button", { name: "Create project" }));
    expect(createProject.mutate).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "Aurora API!",
        github: { owner: "jane", name: "Aurora-API", private: true },
      }),
      expect.anything(),
    );
  });

  it("refuses to name a GitHub repo after an unusable project name", () => {
    const { createProject } = setupCreate();
    fireEvent.change(screen.getByPlaceholderText("my-project"), { target: { value: "!!!" } });
    expect(screen.getByText(/can't name a repository after/)).toBeDefined();
    next();
    fireEvent.click(screen.getByRole("button", { name: "Create project" }));
    expect(createProject.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ github: null }),
      expect.anything(),
    );
  });
});

describe("AddProjectDialog clone location default", () => {
  function setupCloneReporting(defaultReposRoot: string | undefined) {
    const hooks = setupHooks();
    // Must precede the render: the prefill runs in an effect keyed on this.
    q.useBootstrap.mockReturnValue({
      data: { config: { state: "ready" }, settings: {}, defaultReposRoot },
    });
    render(<AddProjectDialog open onClose={vi.fn()} onAdded={vi.fn()} />);
    fireEvent.change(screen.getByPlaceholderText("https://github.com/user/thoughts.git"), {
      target: { value: "https://github.com/acme/widgets.git" },
    });
    return hooks;
  }

  const values = () => screen.getAllByRole("textbox").map((i) => (i as HTMLInputElement).value);

  it("prefills the location the server reports, whichever machine that is", async () => {
    // A Linux root while this client runs on macOS: the point is that the
    // answer comes from the machine the clone lands on. This used to be
    // `join(homeDir(), "hyprlayer", "repos")` — *this* machine's home — which
    // against a remote host prefilled `/Users/...` into a Linux server and
    // failed at its unwritable root with a bare "Permission denied (os error
    // 13)" naming no path.
    q.useConnectionStatus.mockReturnValue({
      data: { host: { destination: "nexus", label: "Nexus" } },
    });
    setupCloneReporting("/home/jt/hyprlayer/repos");
    await vi.waitFor(() => expect(values()).toContain("/home/jt/hyprlayer/repos"));
  });

  it("offers nothing rather than a guess when the server reports no root", async () => {
    q.useConnectionStatus.mockReturnValue({ data: { host: null } });
    setupCloneReporting(undefined);
    await new Promise((r) => setTimeout(r, 0));
    expect(values().every((v) => !v.includes("hyprlayer/repos"))).toBe(true);
  });
});
