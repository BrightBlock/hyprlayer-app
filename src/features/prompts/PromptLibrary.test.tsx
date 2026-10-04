// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useState } from "react";

import { PromptLibrary } from "./PromptLibrary";
import type { WorkshopChrome } from "@/components/workshop";
import type { AppSettings } from "@/lib/bindings";

const chrome: WorkshopChrome = {
  sections: [],
  active: "prompts",
  onSection: vi.fn(),
  scope: null,
  footer: null,
};

function Harness({ settings }: { settings: AppSettings }) {
  const [selectedId, setSelectedId] = useState("commit");
  return (
    <PromptLibrary
      chrome={chrome}
      settings={settings}
      selectedId={selectedId}
      onSelect={setSelectedId}
      filter="all"
      onFilter={vi.fn()}
    />
  );
}

const mutate = vi.fn();
const confirmMock = vi.fn(async (..._args: unknown[]) => true);

vi.mock("@/lib/useViewport", () => ({
  useViewport: () => ({ width: 1280, narrow: false, compact: false }),
}));

vi.mock("../../lib/queries", () => ({
  invalidateBootstrap: vi.fn(),
  useSetAppSettings: () => ({ mutate, isError: false, error: null }),
  useAllWorkstreams: () => ({ data: [] }),
  useWorkstreamPromptContext: () => ({ data: undefined, isError: false, isLoading: false }),
  useBootstrap: () => ({
    data: {
      settings: {},
      promptDefaults: {
        commit: "/commit",
        ship: "Push and open a PR against `{{base_branch}}`.",
      },
      projects: [
        { path: "/repos/app", mappedName: "app" },
        { path: "/repos/cli", mappedName: "cli" },
      ],
    },
  }),
}));

const saveDialog = vi.fn(async () => "/Users/jt/Desktop/share" as string | null);
const openDialog = vi.fn(async () => "/Users/jt/Desktop/share.hlprompts" as string | null);

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: (...args: unknown[]) => confirmMock(...args),
  save: (...args: unknown[]) => saveDialog(...(args as [])),
  open: (...args: unknown[]) => openDialog(...(args as [])),
}));

// Phase 6 moved the file I/O to the client and the prompts themselves over the
// wire, so the flow is now five calls across two machines. Nothing here covered
// it — an audit on 2026-09-16 found this file never mocked `commands` at all and
// never reached the import/export code, so it passed without exercising it.
const ipc = vi.hoisted(() => ({
  exportPrompts: vi.fn(),
  writePromptExport: vi.fn(),
  readPromptImport: vi.fn(),
  previewPromptImport: vi.fn(),
  applyPromptImport: vi.fn(),
}));

vi.mock("@/lib/bindings", () => ({ commands: ipc }));

const PROMPTS_JSON = JSON.stringify({
  hlprompts: 1,
  prompts: [{ name: "Imported", body: "b" }],
});

afterEach(() => {
  cleanup();
  mutate.mockReset();
  confirmMock.mockClear();
  saveDialog.mockClear();
  openDialog.mockClear();
  for (const fn of Object.values(ipc)) fn.mockReset();
});

const EMPTY: AppSettings = {};

function bodyTextarea(): HTMLTextAreaElement {
  return screen.getByPlaceholderText(
    "What should the agent do when this prompt runs?",
  ) as HTMLTextAreaElement;
}

describe("PromptLibrary", () => {
  it("selects Commit by default and shows its stock body from promptDefaults", () => {
    render(<Harness settings={EMPTY} />);
    expect(screen.getAllByText("Built-in").length).toBeGreaterThan(0);
    expect(bodyTextarea().value).toBe("/commit");
    expect(screen.queryByText("Modified")).toBeNull();
    expect(screen.queryByText("Reset to default")).toBeNull();
  });

  it("editing a built-in body persists it as an override on blur", () => {
    render(<Harness settings={EMPTY} />);
    fireEvent.change(bodyTextarea(), { target: { value: "/commit\n\nNever use chore:." } });
    fireEvent.blur(bodyTextarea());
    expect(mutate).toHaveBeenCalledTimes(1);
    const saved = mutate.mock.calls[0][0] as AppSettings;
    expect(saved.prompts?.builtinBodies?.commit).toBe("/commit\n\nNever use chore:.");
    expect(screen.getAllByText("Modified").length).toBeGreaterThan(0);
  });

  it("Reset to default drops the built-in override", () => {
    const settings: AppSettings = {
      prompts: { builtinBodies: { commit: "custom text" }, custom: [] },
    };
    render(<Harness settings={settings} />);
    expect(bodyTextarea().value).toBe("custom text");
    fireEvent.click(screen.getByText("Reset to default"));
    const saved = mutate.mock.calls[mutate.mock.calls.length - 1][0] as AppSettings;
    expect(saved.prompts?.builtinBodies?.commit).toBeUndefined();
    expect(bodyTextarea().value).toBe("/commit");
  });

  it("clearing a built-in body drops the override key on save, keeping the draft empty", () => {
    const settings: AppSettings = {
      prompts: { builtinBodies: { commit: "custom text" }, custom: [] },
    };
    render(<Harness settings={settings} />);
    fireEvent.change(bodyTextarea(), { target: { value: "  " } });
    fireEvent.blur(bodyTextarea());
    const saved = mutate.mock.calls[mutate.mock.calls.length - 1][0] as AppSettings;
    expect(saved.prompts?.builtinBodies?.commit).toBeUndefined();
    expect(bodyTextarea().value).toBe("  ");
  });

  it("ship's stock body comes from promptDefaults (template form)", () => {
    render(<Harness settings={EMPTY} />);
    fireEvent.click(screen.getByText("Ship"));
    expect(bodyTextarea().value).toContain("{{base_branch}}");
  });

  it("adds a custom prompt, persists immediately, and selects it", () => {
    render(<Harness settings={EMPTY} />);
    fireEvent.click(screen.getByRole("button", { name: "New prompt" }));
    const saved = mutate.mock.calls[mutate.mock.calls.length - 1][0] as AppSettings;
    expect(saved.prompts?.custom).toHaveLength(1);
    expect(saved.prompts?.custom?.[0].name).toBe("Untitled prompt");
    expect(screen.getByLabelText("Prompt name")).toBeTruthy();
    expect(screen.getByLabelText("Pin to composer")).toBeTruthy();
  });

  it("pin toggle persists the pinned flag", () => {
    const settings: AppSettings = {
      prompts: {
        custom: [{ id: "p1", name: "Tests", body: "b", overrides: {}, chord: null, pinned: false }],
      },
    };
    render(<Harness settings={settings} />);
    fireEvent.click(screen.getByText("Tests"));
    fireEvent.click(screen.getByLabelText("Pin to composer"));
    const saved = mutate.mock.calls[mutate.mock.calls.length - 1][0] as AppSettings;
    expect(saved.prompts?.custom?.[0].pinned).toBe(true);
  });

  it("insert variable appends the token to the body", () => {
    const settings: AppSettings = {
      prompts: {
        custom: [{ id: "p1", name: "Tests", body: "Cover", overrides: {}, chord: null, pinned: false }],
      },
    };
    render(<Harness settings={settings} />);
    fireEvent.click(screen.getByText("Tests"));
    fireEvent.click(screen.getByText("Insert variable"));
    fireEvent.click(screen.getByText("{{changed_files}}"));
    expect(bodyTextarea().value).toContain("{{changed_files}}");
  });

  it("records a chord for a custom prompt and persists it on the prompt", () => {
    const settings: AppSettings = {
      prompts: {
        custom: [{ id: "p1", name: "Tests", body: "b", overrides: {}, chord: null, pinned: false }],
      },
    };
    render(<Harness settings={settings} />);
    fireEvent.click(screen.getByText("Tests"));
    fireEvent.click(screen.getByText("Edit"));
    fireEvent.keyDown(window, { key: "9", ctrlKey: true, shiftKey: true });
    fireEvent.click(screen.getByText("Save"));
    const saved = mutate.mock.calls[mutate.mock.calls.length - 1][0] as AppSettings;
    expect(saved.prompts?.custom?.[0].chord).toBe("mod+shift+9");
  });

  it("records a built-in chord as its catalog command's keybinding override", () => {
    render(<Harness settings={EMPTY} />);
    fireEvent.click(screen.getByText("Edit"));
    fireEvent.keyDown(window, { key: "9", ctrlKey: true });
    fireEvent.click(screen.getByText("Save"));
    const saved = mutate.mock.calls[mutate.mock.calls.length - 1][0] as AppSettings;
    expect(saved.keybindings?.overrides?.["ws.commit"]).toBe("mod+9");
  });

  it("adds a project override seeded from the global body", () => {
    const settings: AppSettings = {
      prompts: {
        custom: [
          { id: "p1", name: "Tests", body: "global body", overrides: {}, chord: null, pinned: false },
        ],
      },
    };
    render(<Harness settings={settings} />);
    fireEvent.click(screen.getByText("Tests"));
    fireEvent.click(screen.getByText("Project override…"));
    fireEvent.click(screen.getByText("app"));
    const saved = mutate.mock.calls[mutate.mock.calls.length - 1][0] as AppSettings;
    expect(saved.prompts?.custom?.[0].overrides?.["/repos/app"]).toBe("global body");
    expect(screen.getByText(/Editing the override used only in app/)).toBeTruthy();
    expect(screen.getByText("Remove override")).toBeTruthy();
  });

  it("deletes a custom prompt after confirmation", async () => {
    const settings: AppSettings = {
      prompts: {
        custom: [{ id: "p1", name: "Tests", body: "b", overrides: {}, chord: null, pinned: false }],
      },
    };
    render(<Harness settings={settings} />);
    fireEvent.click(screen.getByText("Tests"));
    fireEvent.click(screen.getByText("Delete"));
    await vi.waitFor(() => expect(mutate).toHaveBeenCalled());
    const saved = mutate.mock.calls[mutate.mock.calls.length - 1][0] as AppSettings;
    expect(saved.prompts?.custom).toHaveLength(0);
  });

  it("exports by asking the server for content and writing it on the client", async () => {
    ipc.exportPrompts.mockResolvedValue({
      status: "ok",
      data: { content: PROMPTS_JSON, promptCount: 1 },
    });
    ipc.writePromptExport.mockResolvedValue({
      status: "ok",
      data: "/Users/jt/Desktop/share.hlprompts",
    });
    render(<Harness settings={EMPTY} />);

    // Opens the export dialog; its own confirm button is the one that fires.
    fireEvent.click(screen.getByRole("button", { name: "Export prompts" }));
    const confirmExport = await vi.waitFor(() => {
      const hit = screen
        .getAllByRole("button", { name: /^Export( \d+)?$/ })
        .filter((b) => b.getAttribute("aria-label") !== "Export prompts");
      if (hit.length !== 1) throw new Error(`expected one confirm button, saw ${hit.length}`);
      return hit[0];
    });
    fireEvent.click(confirmExport);

    await vi.waitFor(() => expect(ipc.writePromptExport).toHaveBeenCalled());
    // The server builds the bytes and never sees a path; the client writes them.
    expect(ipc.exportPrompts.mock.calls[0][0]).not.toHaveProperty("path");
    expect(ipc.writePromptExport).toHaveBeenCalledWith(
      "/Users/jt/Desktop/share",
      PROMPTS_JSON,
    );
  });

  it("imports by reading the file on the client and sending its text, not its path", async () => {
    ipc.readPromptImport.mockResolvedValue({ status: "ok", data: PROMPTS_JSON });
    ipc.previewPromptImport.mockResolvedValue({
      status: "ok",
      data: {
        sourceSha256: "abc",
        sourceVersion: 1,
        source: null,
        prompts: [
          {
            key: "0",
            name: "Imported",
            body: "b",
            chord: null,
            chordTaken: false,
            unknownProjects: [],
          },
        ],
        conflicts: [],
      },
    });
    render(<Harness settings={EMPTY} />);

    fireEvent.click(screen.getByRole("button", { name: "Import prompts" }));

    await vi.waitFor(() => expect(ipc.previewPromptImport).toHaveBeenCalled());
    expect(ipc.readPromptImport).toHaveBeenCalledWith("/Users/jt/Desktop/share.hlprompts");
    // The path stops at the client boundary: the server is handed the contents.
    expect(ipc.previewPromptImport.mock.calls[0][0]).toBe(PROMPTS_JSON);
  });

  it("surfaces a client-side read failure instead of previewing nothing", async () => {
    ipc.readPromptImport.mockResolvedValue({
      status: "error",
      error: { code: "prompts_file_too_large", message: "That file is too large.", remedy: null },
    });
    render(<Harness settings={EMPTY} />);

    fireEvent.click(screen.getByRole("button", { name: "Import prompts" }));

    await vi.waitFor(() => expect(ipc.readPromptImport).toHaveBeenCalled());
    expect(ipc.previewPromptImport).not.toHaveBeenCalled();
  });
});
