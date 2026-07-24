// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { RepoPathStatus, ThoughtsProfile } from "@/lib/bindings";
import { AddProjectDialog } from "./AddProjectDialog";

// Mock the data hooks so the dialog runs its real logic against plain values —
// no react-query, no Tauri IPC. `vi.hoisted` lets the hoisted vi.mock factory
// reach them.
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
}));
vi.mock("@/lib/queries", () => q);
// Identity debounce so the path probe resolves synchronously in the test.
vi.mock("@/lib/useDebounced", () => ({ useDebounced: (v: unknown) => v }));
vi.mock("@tauri-apps/api/path", () => ({
  homeDir: () => Promise.resolve("/home"),
  join: (...p: string[]) => Promise.resolve(p.join("/")),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

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
) {
  const clone = mutation();
  const importProject = mutation();
  const importFolder = mutation();
  const createProfile = mutation();
  q.useBootstrap.mockReturnValue({
    data: { config: { state: configState }, settings: { reposRoot: "/repos" } },
  });
  q.useCloneProject.mockReturnValue(clone);
  q.useImportProject.mockReturnValue(importProject);
  q.useImportLocalFolder.mockReturnValue(importFolder);
  q.useDefaultUsername.mockReturnValue({ data: "jane" });
  q.useRepoPathInfo.mockReturnValue({ data: undefined });
  q.useRecentRepos.mockReturnValue({ data: [], isLoading: false });
  q.useThoughtsProfiles.mockReturnValue({ data: profiles, isLoading: false, error: null });
  q.useCreateThoughtsProfile.mockReturnValue(createProfile);
  return { clone, importProject, importFolder, createProfile };
}

const next = () => fireEvent.click(screen.getByRole("button", { name: "Next" }));

/** Open a DropdownSelect by its Field label and pick an option by its text.
 *  Asserts the system-select contract: picking a row closes the menu. */
function pick(field: string, option: string) {
  fireEvent.click(screen.getByLabelText(field));
  fireEvent.click(screen.getByRole("option", { name: option }));
  expect(screen.queryByRole("listbox")).toBeNull();
}

/** Render in disk mode with a mocked classification for the browsed folder. */
function setupDisk(status: RepoPathStatus | undefined, configState: "ok" | "missing" = "ok") {
  const hooks = setupHooks([defaultProfile], configState);
  q.useRepoPathInfo.mockReturnValue({ data: status });

  render(<AddProjectDialog open onClose={vi.fn()} onAdded={vi.fn()} />);
  // Switch to the "Open from disk" tab and type a path.
  fireEvent.click(screen.getByRole("button", { name: "Open from disk" }));
  fireEvent.change(screen.getByPlaceholderText("/path/to/repo-or-folder"), {
    target: { value: "/my/folder" },
  });
  return hooks;
}

/** Render in clone mode with a URL typed, ready to advance to the Thoughts step. */
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
    // Plain folder is the non-mutating default; git init is the opt-in.
    expect(
      screen.getByRole("switch", { name: /Set up a git repository/ }).getAttribute("aria-checked"),
    ).toBe("false");
    expect(screen.getByRole("button", { name: "Add folder" })).toBeDefined();
  });

  it("initializes with the chosen branch when git setup is toggled on", () => {
    const { importProject } = setupDisk("nonGit");
    fireEvent.click(screen.getByRole("switch", { name: /Set up a git repository/ }));
    // Override the default branch name (a step-1 field).
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
    // Next is gated (step1Ok is false), so the Thoughts step never renders.
    const nextButton = screen.getByRole("button", { name: "Next" }) as HTMLButtonElement;
    expect(nextButton.disabled).toBe(true);
    fireEvent.click(nextButton);
    expect(screen.queryByRole("button", { name: "Add project" })).toBeNull();
    expect(importProject.mutate).not.toHaveBeenCalled();
  });

  it("adds a non-git folder as-is, skipping the thoughts step entirely", () => {
    const { importFolder, importProject } = setupDisk("nonGit");
    // Folder-first: no git setup to configure, no second step — step 1 submits,
    // and nothing in the dialog calls the target a repo.
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

  it("needs no bootstrap username for a folder — it writes no config", () => {
    // A true first run (no hyprlayer config yet). The repo paths demand a
    // username to bootstrap one; a folder project never touches the config.
    const { importFolder } = setupDisk("nonGit", "missing");
    // Folder-first: no username demanded. Opting into git brings it back.
    // (The Field's <label> also wraps its hint text, so match loosely.)
    expect(screen.queryByLabelText(/Your name/)).toBeNull();
    fireEvent.click(screen.getByRole("switch", { name: /Set up a git repository/ }));
    expect(screen.getByLabelText(/Your name/)).toBeDefined();
    fireEvent.click(screen.getByRole("switch", { name: /Set up a git repository/ }));
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
    expect(importFolder.mutate).toHaveBeenCalled();
  });

  it("drops the folder offer if the path turns out to be a git repo", () => {
    const { importFolder } = setupDisk("nonGit");
    // The debounced probe lands: it's a real repo after all. The choice is gone
    // and the wizard is back to its two-step self — no stale folder submit.
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
    // Which command a disk path submits to depends on the classification —
    // folder-first means a pre-probe submit could silently git-init a plain
    // folder, so Next stays gated until the probe lands.
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
    // Auto folder derived from the URL, marked new (not in the profile's folders).
    expect(screen.getByText("~/thoughts/repos/widgets/")).toBeDefined();
    expect(screen.getByText("new")).toBeDefined();
    // Back returns to the repository step with state intact.
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
    // The preview follows the profile, and the auto folder now exists there.
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
    // e.g. a permission-denied repos dir: the backend reports why, and the
    // dialog warns next to the folder select rather than silently offering
    // only "new" folders.
    setupClone([{ ...defaultProfile, foldersError: "/Users/x/thoughts/repos: Permission denied" }]);
    next();
    expect(screen.getByText(/Couldn't read existing folders/)).toBeDefined();
    expect(screen.getByText(/Permission denied/)).toBeDefined();
  });

  it("requires a name for a new folder and passes it through", () => {
    const { clone } = setupClone();
    next();
    pick("Folder", "New folder…");
    // Empty custom name gates submit.
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
    // The full wizard still runs: step 1 → thoughts step → submit.
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
    // The inline form gates the main submit while open.
    const submit = screen.getByRole("button", { name: "Clone repo" }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Profile name"), { target: { value: "team" } });
    fireEvent.click(screen.getByRole("button", { name: "Create profile" }));
    expect(createProfile.mutate).toHaveBeenCalledWith(
      { name: "team", thoughtsRepo: null },
      expect.anything(),
    );
    // Success closes the form and unblocks submit.
    expect(screen.queryByLabelText("Profile name")).toBeNull();
    expect(submit.disabled).toBe(false);
  });
});
