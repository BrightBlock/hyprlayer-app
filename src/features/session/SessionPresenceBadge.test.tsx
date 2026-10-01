// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { ClientRef } from "@/lib/bindings";
import { SessionPresenceBadge } from "./SessionPresenceBadge";
import { useTranscriptSync } from "./syncStore";

const MAC: ClientRef = { clientId: "a", device: "jons-macbook-pro" };
const MINI: ClientRef = { clientId: "b", device: "studio-mini" };

afterEach(() => {
  cleanup();
  useTranscriptSync.setState({ viewers: {} });
});

const view = (viewers: ClientRef[]) => useTranscriptSync.setState({ viewers: { s1: viewers } });

describe("SessionPresenceBadge", () => {
  it("stays hidden with no session, no viewers, or only this client", () => {
    const { container, rerender } = render(<SessionPresenceBadge sessionId={undefined} />);
    expect(container.textContent).toBe("");
    rerender(<SessionPresenceBadge sessionId="s1" />);
    expect(container.textContent).toBe("");
    act(() => view([MAC]));
    expect(container.textContent).toBe("");
  });

  it("shows initials and the count once a second client views the session", () => {
    view([MAC, MINI]);
    render(<SessionPresenceBadge sessionId="s1" />);
    expect(screen.getByText("2 clients")).toBeTruthy();
    expect(screen.getByText("JM")).toBeTruthy();
    expect(screen.getByText("SM")).toBeTruthy();
  });

  it("hides again when one of two leaves", () => {
    view([MAC, MINI]);
    const { container } = render(<SessionPresenceBadge sessionId="s1" />);
    act(() => view([MAC]));
    expect(container.textContent).toBe("");
  });
});
