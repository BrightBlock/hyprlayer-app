// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { CatalogProvider, OpencodeSignInStatus } from "@/lib/bindings";
import { ProviderPickerModal } from "./ProviderPickerModal";

const m = vi.hoisted(() => ({
  status: undefined as OpencodeSignInStatus | undefined,
  begin: vi.fn(),
  cancel: vi.fn(),
  reauth: vi.fn(),
}));
vi.mock("@/lib/queries", () => ({
  useSetOpencodeProviderKey: () => ({ mutateAsync: vi.fn(), isPending: false, isError: false }),
  useBeginOpencodeSignIn: () => ({ mutateAsync: m.begin, isPending: false, isError: false }),
  useCompleteOpencodeSignIn: () => ({ mutate: vi.fn(), isPending: false, isError: false }),
  useOpencodeSignInStatus: (_provider: string, attemptId: string | null) => ({
    data: attemptId ? m.status : undefined,
    isError: false,
  }),
  cancelOpencodeSignIn: m.cancel,
}));
vi.mock("@/lib/browser", () => ({ openExternalUrl: vi.fn() }));
vi.mock("@/lib/reauth", () => ({ reauthSucceeded: m.reauth }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  m.status = undefined;
});

const XAI: CatalogProvider = { id: "xai", name: "xAI", env: ["XAI_API_KEY"], doc: null, models: [] };
const METHOD = { provider: "xai", methodId: "device", label: "Sign in with xAI" };
const ATTEMPT = {
  provider: "xai",
  attemptId: "con_1",
  url: "https://example.invalid/device",
  instructions: "Enter code: STUB-CODE",
  needsCode: false,
  acceptsPaste: false,
};

async function openSignIn() {
  const onClose = vi.fn();
  const view = render(
    <ProviderPickerModal open onClose={onClose} providers={[XAI]} configured={new Set()} signInMethods={[METHOD]} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "xAI" }));
  fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
  m.begin.mockResolvedValue(ATTEMPT);
  m.status = { state: "pending" };
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Sign in with xAI" })));
  expect(screen.getByText("Enter code: STUB-CODE")).toBeTruthy();
  return { onClose, view };
}

function dismissSignIn() {
  const backdrop = screen.getByRole("dialog", { name: "Sign in to xAI" }).parentElement!;
  fireEvent.mouseDown(backdrop);
}

describe("ProviderPickerModal sign-in", () => {
  it("cancelling the sign-in returns to the provider and cancels the attempt", async () => {
    const { onClose } = await openSignIn();
    dismissSignIn();
    expect(screen.queryByRole("dialog", { name: "Sign in to xAI" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "Providers" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Sign in" })).toBeTruthy();
    expect(m.cancel).toHaveBeenCalledWith("xai", "con_1");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("a finished sign-in closes the picker", async () => {
    const { onClose, view } = await openSignIn();
    m.status = { state: "complete" };
    view.rerender(
      <ProviderPickerModal open onClose={onClose} providers={[XAI]} configured={new Set()} signInMethods={[METHOD]} />,
    );
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(m.reauth).toHaveBeenCalledWith("opencode");
    expect(m.cancel).not.toHaveBeenCalled();
  });
});
