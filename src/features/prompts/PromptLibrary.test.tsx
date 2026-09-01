// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useState } from "react";

import { PromptLibrary } from "./PromptLibrary";
import type { AppSettings } from "@/lib/bindings";

function Harness({ settings }: { settings: AppSettings }) {
  const [selectedId, setSelectedId] = useState("commit");
  return <PromptLibrary settings={settings} selectedId={selectedId} onSelect={setSelectedId} />;
}

const mutate = vi.fn();
const confirmMock = vi.fn(async (..._args: unknown[]) => true);

vi.mock("../../lib/queries", () => ({
  invalidateBootstrap: vi.fn(),
  useSetAppSettings: () => ({ mutate, isError: false, error: null }),
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

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: (...args: unknown[]) => confirmMock(...args),
}));

afterEach(() => {
  cleanup();
  mutate.mockReset();
  confirmMock.mockClear();
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
    expect(screen.getByText("Modified")).toBeTruthy();
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
    fireEvent.click(screen.getByLabelText("New prompt"));
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
});
