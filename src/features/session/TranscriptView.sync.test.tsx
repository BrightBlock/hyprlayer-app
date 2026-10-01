// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { ClientRef } from "@/lib/bindings";
import type { TranscriptItem } from "./transcript";
import { TranscriptView, type TranscriptSync } from "./TranscriptView";

afterEach(cleanup);

const OTHER: ClientRef = { clientId: "them", device: "jons-macbook-pro" };
const NONE: TranscriptSync = { tail: null, divider: null, freshFromSeq: null, selfClientId: "me" };

function user(id: string, seq: number, author: ClientRef | null): TranscriptItem {
  return {
    kind: "user",
    id,
    text: `prompt ${id}`,
    stage: null,
    attachments: [],
    createdAt: null,
    promptName: null,
    author,
    seq,
  };
}

const reply = (id: string, seq: number): TranscriptItem => ({
  kind: "assistant",
  id,
  text: `reply ${id}`,
  streaming: false,
  seq,
});

function renderSync(items: TranscriptItem[], sync: Partial<TranscriptSync>, workingSince: number | null = null) {
  return render(
    <TranscriptView
      items={items}
      busy={false}
      connecting={false}
      harnessLabel="Claude Code"
      workingSince={workingSince}
      permission={{ onAllow: () => {}, onAllowForSession: () => {}, onDeny: () => {} }}
      prompt={{ onRewind: () => {} }}
      sync={{ ...NONE, ...sync }}
    />,
  );
}

describe("remote user-turn chip", () => {
  it("marks another device's turn, never this client's own", () => {
    renderSync([user("a", 1, { clientId: "me", device: "my-mac" }), user("b", 3, OTHER)], {});
    expect(screen.getByText(/from jons-macbook-pro/)).toBeTruthy();
    expect(screen.queryByText(/from my-mac/)).toBeNull();
  });

  it("marks nothing while this client's identity is unknown", () => {
    renderSync([user("b", 3, OTHER)], { selfClientId: null });
    expect(screen.queryByText(/from jons-macbook-pro/)).toBeNull();
  });
});

describe("new-from divider", () => {
  it("sits before the first item at or past its seq", () => {
    const { container } = renderSync([user("a", 1, null), reply("r", 2), user("b", 5, OTHER), reply("s", 6)], {
      divider: { seq: 4, device: "jons-macbook-pro" },
    });
    const text = container.textContent ?? "";
    expect(text.indexOf("New from jons-macbook-pro")).toBeGreaterThan(text.indexOf("reply r"));
    expect(text.indexOf("New from jons-macbook-pro")).toBeLessThan(text.indexOf("prompt b"));
  });

  it("fades in only the items the pull added", () => {
    const { container } = renderSync([reply("r", 2), reply("s", 6)], { freshFromSeq: 6 });
    expect(container.querySelectorAll(".animate-sync-in")).toHaveLength(1);
    expect(container.querySelector(".animate-sync-in")?.textContent).toContain("reply s");
  });
});

describe("skeleton tail", () => {
  it("draws one placeholder however many turns wait; the count lives in the composer notice", () => {
    const turns = [42, 43, 44, 45, 46].map((rev) => ({ rev, kinds: ["assistant" as const, "tool" as const] }));
    const { container } = renderSync([], { tail: { phase: "guard", turns, count: 5, working: null } });
    const placeholders = container.querySelectorAll("[data-sync-placeholder]");
    expect(placeholders).toHaveLength(1);
    // One exchange: a prompt row and a reply row.
    expect(placeholders[0].querySelectorAll("[data-sync-row]")).toHaveLength(2);
    expect(screen.queryByText(/not shown/)).toBeNull();
    expect(screen.queryByText(/5 turns/)).toBeNull();
  });

  it("shows only the working line, not a placeholder too, while the other device's turn runs", () => {
    const tail = { phase: "behind" as const, turns: [{ rev: 42, kinds: ["user" as const] }], count: 3, working: "nexus" };
    const { container } = renderSync([], { tail });
    expect(container.querySelectorAll("[data-sync-placeholder]")).toHaveLength(0);
    expect(screen.getByText("nexus is working…")).toBeTruthy();
  });

  it("shimmers only while syncing", () => {
    const tail = { turns: [{ rev: 42, kinds: ["assistant" as const] }], count: 1, working: null };
    const { container, rerender } = renderSync([], { tail: { ...tail, phase: "behind" } });
    expect(container.querySelector(".animate-shimmer")).toBeNull();
    rerender(
      <TranscriptView
        items={[]}
        busy={false}
        connecting={false}
        harnessLabel="Claude Code"
        workingSince={null}
        permission={{ onAllow: () => {}, onAllowForSession: () => {}, onDeny: () => {} }}
        prompt={{ onRewind: () => {} }}
        sync={{ ...NONE, tail: { ...tail, phase: "syncing" } }}
      />,
    );
    expect(container.querySelector(".animate-shimmer")).not.toBeNull();
  });

  it("replaces the elapsed counter while another device works", () => {
    const { container } = renderSync(
      [],
      { tail: { phase: "inSync", turns: [], count: 0, working: "Another device" } },
      Date.now(),
    );
    expect(screen.getByText("Another device is working…")).toBeTruthy();
    expect(container.textContent).not.toMatch(/\d+s/);
  });
});
