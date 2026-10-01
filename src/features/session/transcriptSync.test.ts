// @vitest-environment node
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { ClientRef, SessionEvent, SessionLogEntryDto } from "@/lib/bindings";
import { addUserMessage, initialSessionView, reduceEvent, type SessionView } from "./transcript";
import {
  canSend,
  deviceLabel,
  heldSummary,
  mergeEntries,
  RETRY_DELAYS_MS,
  route,
  transition,
  turnAuthorAfter,
  type Arrival,
  type PendingTurn,
  type RouteContext,
  type SyncInput,
  type SyncState,
} from "./transcriptSync";
import { SyncComposerNotice, syncNotice } from "./SyncComposerNotice";

const SELF: ClientRef = { clientId: "me", device: "my-mac" };
const OTHER: ClientRef = { clientId: "them", device: "jons-macbook-pro" };

function synced(over: Partial<SyncState> = {}): SyncState {
  return {
    localRev: 5,
    committedRev: 5,
    remoteRev: 5,
    resetRev: 0,
    phase: "inSync",
    pendingTurns: [],
    authorDevice: null,
    workingDevice: null,
    flushable: false,
    draft: false,
    fetching: false,
    retry: null,
    syncedWith: null,
    gone: false,
    ...over,
  };
}

function remote(over: Partial<Extract<SyncInput, { kind: "remote" }>> = {}): SyncInput {
  const rev = over.rev ?? 6;
  return {
    kind: "remote",
    rev,
    turns: [{ rev, kinds: ["user"] }],
    device: OTHER.device,
    working: false,
    flushable: false,
    attached: false,
    own: false,
    ...over,
  };
}

function fetched(over: Partial<Extract<SyncInput, { kind: "fetched" }>> = {}): SyncInput {
  return { kind: "fetched", rev: 6, resetRev: 0, inFlight: 0, remaining: 0, device: null, ...over };
}

function run(state: SyncState, ...inputs: SyncInput[]) {
  let s = state;
  const effects: string[] = [];
  for (const input of inputs) {
    const out = transition(s, input);
    s = out.state;
    effects.push(...out.effects.map((e) => e.kind));
  }
  return { state: s, effects };
}

const pending = (s: SyncState) => s.remoteRev > s.localRev || s.flushable;

describe("transition", () => {
  it("T1 an own revision shows nothing: attached raises only committedRev", () => {
    const { state, effects } = run(synced(), remote({ own: true, attached: true, device: SELF.device }));
    expect(state.committedRev).toBe(6);
    expect(state.remoteRev).toBe(5);
    expect(state.pendingTurns).toEqual([]);
    expect(state.phase).toBe("inSync");
    expect(effects).toEqual([]);
  });

  it("T1 an own revision on a detached view pulls silently and its fetch sets localRev", () => {
    const start = synced({ draft: true });
    const { state, effects } = run(start, remote({ own: true, device: SELF.device }));
    expect(effects).toEqual(["pull"]);
    expect(state.phase).toBe("inSync");
    expect(state.pendingTurns).toEqual([]);
    expect(state.remoteRev).toBe(5);
    const after = run(state, fetched({ rev: 6 }));
    expect(after.state.localRev).toBe(6);
    expect(after.state.phase).toBe("inSync");
    expect(after.effects).not.toContain("flashSynced");
  });

  it("T2 another client's revision with an empty composer waits behind", () => {
    const { state, effects } = run(synced(), remote());
    expect(effects).toEqual([]);
    expect(state.phase).toBe("behind");
    expect(state.fetching).toBe(false);
    expect(state.pendingTurns).toEqual([{ rev: 6, kinds: ["user"] }]);
  });

  it("T3 another client's revision while drafting guards and never pulls", () => {
    const { state, effects } = run(synced({ draft: true }), remote());
    expect(state.phase).toBe("guard");
    expect(effects).not.toContain("pull");
  });

  it("T5 clearing the draft while behind leaves the guard and does not pull", () => {
    const guarded = run(synced({ draft: true }), remote()).state;
    const { state, effects } = run(guarded, { kind: "composer", draft: false });
    expect(effects).toEqual([]);
    expect(state.phase).toBe("behind");
  });

  it("T6 a revision at or below localRev does not reopen behind", () => {
    const start = synced();
    for (const rev of [4, 5]) {
      const { state, effects } = run(start, remote({ rev, turns: [{ rev, kinds: ["assistant"] }] }));
      expect(state.phase).toBe("inSync");
      expect(state.pendingTurns).toEqual([]);
      expect(effects).toEqual([]);
    }
  });

  it("T7 a revision that lands during a fetch is fetched after the fetch ends", () => {
    const syncing = run(synced(), remote(), { kind: "pull" }).state;
    const during = run(syncing, remote({ rev: 7, turns: [{ rev: 7, kinds: ["assistant"] }] }));
    expect(during.effects).toEqual([]);
    expect(during.state.phase).toBe("syncing");
    const after = run(during.state, fetched({ rev: 6 }));
    expect(after.state.localRev).toBe(6);
    expect(after.effects).toContain("pull");
    expect(after.state.phase).toBe("syncing");
  });

  it("T8 a gap only another client's in-flight send explains waits, then ends when none is in flight", () => {
    const stale = run(synced(), { kind: "stale", rev: 6, committedRev: 5 });
    expect(stale.effects).toEqual([]);
    expect(stale.state.phase).toBe("behind");

    const pulled = run(stale.state, { kind: "pull" }, fetched({ rev: 5, inFlight: 1 }));
    expect(pulled.effects).toEqual(["pull"]);
    expect(pulled.state.phase).toBe("behind");
    expect(pulled.state.remoteRev).toBe(6);
    expect(pulled.state.fetching).toBe(false);

    const drafting = run(pulled.state, { kind: "composer", draft: true });
    expect(drafting.state.phase).toBe("guard");

    const polled = run(pulled.state, { kind: "polled", rev: 5, resetRev: 0, inFlight: 0, working: false });
    expect(polled.state.remoteRev).toBe(5);
    expect(polled.state.phase).toBe("inSync");
    expect(polled.effects).toEqual([]);
  });

  it("T9 a fetch failure keeps the pending turns, goes behind and backs off 1s, 2s, 5s, 15s, 30s, 30s", () => {
    let state = run(synced(), remote(), { kind: "pull" }).state;
    const delays: number[] = [];
    for (let i = 0; i < 6; i += 1) {
      const failed = transition(state, { kind: "fetchFailed" });
      state = failed.state;
      expect(state.phase).toBe("behind");
      expect(state.fetching).toBe(false);
      expect(state.pendingTurns).toEqual([{ rev: 6, kinds: ["user"] }]);
      expect(failed.effects.map((e) => e.kind)).toEqual(["retryIn"]);
      const retry = failed.effects[0] as { kind: "retryIn"; ms: number };
      delays.push(retry.ms);
      expect(state.retry).toEqual({ attempt: i + 1, delayMs: retry.ms });
      state = transition(state, { kind: "pull" }).state;
      expect(state.phase).toBe("syncing");
    }
    expect(delays).toEqual([1_000, 2_000, 5_000, 15_000, 30_000, 30_000]);
    expect(RETRY_DELAYS_MS).toEqual([1_000, 2_000, 5_000, 15_000, 30_000]);
  });

  it("T10 a success after failures resets the backoff", () => {
    const failing = run(synced(), remote(), { kind: "pull" }, { kind: "fetchFailed" }, { kind: "pull" }, { kind: "fetchFailed" });
    expect(failing.state.retry?.attempt).toBe(2);
    const ok = run(failing.state, { kind: "pull" }, fetched({ rev: 6 }));
    expect(ok.state.retry).toBeNull();
    const nextFailure = transition(run(ok.state, remote({ rev: 7 })).state, { kind: "fetchFailed" });
    expect(nextFailure.state.retry).toEqual({ attempt: 1, delayMs: 1_000 });
  });

  it("T11 a 409 while drafting guards with the refusal's counts", () => {
    const { state } = run(synced({ draft: true }), { kind: "stale", rev: 9, committedRev: 8 });
    expect(state.phase).toBe("guard");
    expect(state.remoteRev).toBe(9);
    expect(state.committedRev).toBe(8);
    expect(canSend(state)).toBe(false);
  });

  it("T12 a reset reloads and keeps pending state until the reload is loaded", () => {
    const behind = run(synced(), remote()).state;
    const reset = run(behind, { kind: "reset" });
    expect(reset.effects).toEqual(["reload"]);
    expect(reset.state.pendingTurns).toEqual(behind.pendingTurns);
    expect(reset.state.remoteRev).toBe(6);
    expect(reset.state.phase).toBe("syncing");

    const loaded = run(reset.state, { kind: "loaded", rev: 6, resetRev: 6 });
    expect(loaded.state.localRev).toBe(6);
    expect(loaded.state.resetRev).toBe(6);
    expect(loaded.state.pendingTurns).toEqual([]);
    expect(loaded.state.fetching).toBe(false);
    expect(loaded.state.phase).toBe("inSync");
  });

  it("T13 a pull while already fetching starts no second fetch", () => {
    const syncing = run(synced(), remote(), { kind: "pull" }).state;
    const { state, effects } = run(syncing, { kind: "pull" }, remote({ rev: 7 }), { kind: "composer", draft: false });
    expect(effects).toEqual([]);
    expect(state.fetching).toBe(true);
  });

  it("T14 a deleted session shows nothing pending, and only a reload revives it", () => {
    const behind = run(synced(), remote(), { kind: "stale", rev: 9, committedRev: 8 }).state;
    const gone = run(behind, { kind: "gone" });
    expect(gone.state.phase).toBe("inSync");
    expect(gone.state.pendingTurns).toEqual([]);
    expect(pending(gone.state)).toBe(false);
    expect(gone.effects).toEqual([]);

    const ignored = run(gone.state, remote({ rev: 10 }), { kind: "pull" }, { kind: "composer", draft: false });
    expect(ignored.effects).toEqual([]);
    expect(pending(ignored.state)).toBe(false);

    const revived = run(gone.state, { kind: "loaded", rev: 10, resetRev: 0 });
    expect(revived.state.gone).toBe(false);
  });

  it("T15 canSend is false while syncing or guarded, and in behind", () => {
    expect(canSend(synced())).toBe(true);
    expect(canSend(synced({ phase: "syncing" }))).toBe(false);
    expect(canSend(synced({ phase: "guard" }))).toBe(false);
    expect(canSend(synced({ phase: "behind" }))).toBe(false);
  });

  it("T16 Synced with <device> flashes only after a pull that brought another device's turns", () => {
    const real = run(synced(), remote(), { kind: "pull" }, fetched({ rev: 6, device: OTHER.device }));
    expect(real.effects).toEqual(["pull", "flashSynced"]);
    expect(real.state.syncedWith).toBe(OTHER.device);

    const fromSummary = run(synced(), remote(), { kind: "pull" }, fetched({ rev: 6 }));
    expect(fromSummary.effects).toContain("flashSynced");
    expect(fromSummary.state.syncedWith).toBe(OTHER.device);

    const own = run(synced(), remote({ own: true, device: SELF.device }), fetched({ rev: 6 }));
    expect(own.effects).not.toContain("flashSynced");
    expect(own.state.syncedWith).toBeNull();

    const empty = run(synced(), { kind: "pull" }, fetched({ rev: 5 }));
    expect(empty.effects).toEqual(["pull"]);
    expect(empty.state.syncedWith).toBeNull();
  });

  it("T17 held items flushable with no new rev clear on the pull that applies them", () => {
    const { state, effects } = run(synced(), remote({ rev: 5, turns: [], flushable: true }), { kind: "pull" });
    expect(effects).toEqual(["pull"]);
    expect(state.phase).toBe("syncing");
    const done = run(state, fetched({ rev: 5 }));
    expect(done.state.flushable).toBe(false);
    expect(done.state.phase).toBe("inSync");
  });

  it("T18 a turnless revision while an in-flight send is counted rechecks", () => {
    const stale = run(synced(), { kind: "stale", rev: 6, committedRev: 5 }).state;
    expect(run(stale, remote({ rev: 5, turns: [], device: null })).effects).toEqual(["recheck"]);
    expect(run(stale, remote({ rev: 5, turns: [], attached: true, device: null })).effects).toEqual(["recheck"]);
    expect(run(synced(), remote({ rev: 5, turns: [], device: null })).effects).toEqual([]);
  });

  it("T19 an authoritative read never lowers remoteRev below a reported commit", () => {
    const behind = run(synced(), remote({ rev: 8, turns: [{ rev: 8, kinds: ["user"] }] })).state;
    const polled = run(behind, { kind: "polled", rev: 6, resetRev: 0, inFlight: 0, working: false });
    expect(polled.state.remoteRev).toBe(8);
    expect(polled.state.committedRev).toBe(8);

    const stale = run(synced(), { kind: "stale", rev: 9, committedRev: 7 }).state;
    const read = run(stale, { kind: "pull" }, fetched({ rev: 6 }));
    expect(read.state.committedRev).toBe(7);
    expect(read.state.remoteRev).toBeGreaterThanOrEqual(7);
  });

  it("T20 an attached view's foreign revision raises only committedRev", () => {
    const { state, effects } = run(synced(), remote({ attached: true, working: true }));
    expect(state.committedRev).toBe(6);
    expect(state.remoteRev).toBe(5);
    expect(state.pendingTurns).toEqual([]);
    expect(state.phase).toBe("inSync");
    expect(effects).toEqual([]);
    expect(state.workingDevice).toBe(OTHER.device);
  });

  it("T21 only a resetRev above the loaded one reloads", () => {
    const loaded = synced({ resetRev: 3 });
    expect(run(loaded, { kind: "polled", rev: 5, resetRev: 4, inFlight: 0, working: false }).effects).toEqual(["reload"]);
    expect(run(loaded, { kind: "polled", rev: 5, resetRev: 3, inFlight: 0, working: false }).effects).toEqual([]);
    expect(run(loaded, { kind: "polled", rev: 5, resetRev: 2, inFlight: 0, working: false }).effects).toEqual([]);
    const fetching = synced({ resetRev: 3, fetching: true, phase: "syncing" });
    expect(run(fetching, fetched({ rev: 5, resetRev: 4 })).effects).toEqual(["reload"]);
    expect(run(fetching, fetched({ rev: 5, resetRev: 3 })).effects).toEqual([]);
  });

  it("T22 deviceLabel falls back to another device when no revision named one", () => {
    const { state } = run(synced({ draft: true }), { kind: "stale", rev: 9, committedRev: 8 });
    expect(deviceLabel(state)).toBe("another device");
    expect(deviceLabel(synced({ authorDevice: "" }))).toBe("another device");
    expect(deviceLabel(synced({ workingDevice: "nexus" }))).toBe("nexus");
    expect(deviceLabel(synced({ authorDevice: "mbp", workingDevice: "nexus" }))).toBe("mbp");
  });

  it("a partial page loops without advancing localRev past the rows read", () => {
    const syncing = run(synced(), remote({ rev: 9, turns: [{ rev: 9, kinds: ["user"] }] }), { kind: "pull" }).state;
    const partial = run(syncing, fetched({ rev: 9, remaining: 40 }));
    expect(partial.effects).toEqual(["pull"]);
    expect(partial.state.localRev).toBe(5);
    expect(partial.state.phase).toBe("syncing");
  });
});

function arrival(event: SessionEvent, over: Partial<Arrival> = {}): Arrival {
  return { seq: 10, rev: 6, author: OTHER, event, receivedAt: 0, ...over };
}

const ctx = (over: Partial<RouteContext> = {}): RouteContext => ({
  selfId: SELF.clientId,
  appliedSeq: 5,
  heldCount: 0,
  ownTurnStreaming: false,
  ...over,
});

const userMessage = (text: string, extra: Partial<Extract<SessionEvent, { kind: "userMessage" }>> = {}): SessionEvent => ({
  kind: "userMessage",
  text,
  stage: null,
  ...extra,
});
const assistantText = (text: string): SessionEvent => ({ kind: "assistantText", text, parentId: null });
const thinking = (text: string): SessionEvent => ({ kind: "thinking", text, parentId: null });
const toolUse = (id: string): SessionEvent => ({ kind: "toolUse", id, name: "Read", input: "{}", parentId: null });
const toolResult = (id: string): SessionEvent => ({ kind: "toolResult", toolUseId: id, content: "ok", isError: false, parentId: null });
const turnDone: SessionEvent = { kind: "turnDone", costUsd: null, numTurns: null, contextTokens: null };
const exited: SessionEvent = { kind: "exited", outcome: "completed", code: 0, stderrTail: "" };
const permissionAsk: SessionEvent = { kind: "permissionAsk", requestId: "r1", toolName: "Bash", input: "{}" };
const questionAsk: SessionEvent = { kind: "questionAsk", requestId: "q1", questions: [] };
const rewound: SessionEvent = { kind: "rewound" };

function entry(seq: number | null, event: SessionEvent, createdAt: number | null = 1_000): SessionLogEntryDto {
  return { seq, kind: event.kind, event, createdAt };
}

const after = (seq: number) => ({ kind: "after" as const, seq, createdAt: 1_000 });

describe("route and merge", () => {
  it("R1 an author-less item behind held items waits its turn", () => {
    expect(route(arrival(assistantText("x"), { author: null }), ctx({ heldCount: 2 }))).toBe("hold");
    expect(route(arrival(turnDone, { seq: null, author: null }), ctx({ heldCount: 1 }))).toBe("hold");
  });

  it("R2 another author's partialText is dropped", () => {
    const partial: SessionEvent = { kind: "partialText", text: "tok", parentId: null };
    expect(route(arrival(partial, { seq: null }), ctx())).toBe("drop");
    expect(route(arrival(partial, { seq: null }), ctx({ heldCount: 3 }))).toBe("drop");
    expect(route(arrival(partial, { seq: null, author: SELF }), ctx())).toBe("apply");
  });

  it("R3 another author's permissionAsk with an empty buffer is held; an author-less one applies", () => {
    expect(route(arrival(permissionAsk, { seq: 11 }), ctx())).toBe("hold");
    expect(route(arrival(questionAsk, { seq: null }), ctx())).toBe("hold");
    expect(route(arrival(turnDone, { seq: 12 }), ctx())).toBe("hold");
    expect(route(arrival(permissionAsk, { seq: 11, author: null }), ctx())).toBe("apply");
  });

  it("R4 a buffer is flushable once it holds a commit or ends where the turn waits", () => {
    for (const tail of [permissionAsk, questionAsk, turnDone, exited]) {
      expect(heldSummary([arrival(thinking("t")), arrival(tail, { seq: null })]).flushable).toBe(true);
    }
    expect(heldSummary([arrival(userMessage("hi")), arrival(thinking("t"))]).flushable).toBe(true);
    expect(heldSummary([arrival(thinking("t"))]).flushable).toBe(false);
    expect(heldSummary([arrival(toolUse("u1"))]).flushable).toBe(false);
    expect(heldSummary([]).flushable).toBe(false);
  });

  it("R5 riders join the next commit's revision and flag its tool card", () => {
    const summary = heldSummary([
      arrival(userMessage("hi"), { seq: 10, rev: 6 }),
      arrival(thinking("t"), { seq: 11, rev: 6 }),
      arrival(toolUse("u1"), { seq: 12, rev: 6 }),
      arrival(toolResult("u1"), { seq: 13, rev: 7 }),
      arrival(assistantText("done"), { seq: 14, rev: 8 }),
    ]);
    const turns: PendingTurn[] = [
      { rev: 6, kinds: ["user"] },
      { rev: 7, kinds: ["assistant", "tool"] },
      { rev: 8, kinds: ["assistant"] },
    ];
    expect(summary.turns).toEqual(turns);
    expect(summary.rev).toBe(8);
    expect(summary.device).toBe(OTHER.device);
    expect(summary.working).toBe(true);

    const trailing = heldSummary([arrival(userMessage("hi"), { rev: 6 }), arrival(toolUse("u2"), { seq: 11, rev: 6 })]);
    expect(trailing.turns).toEqual([{ rev: 6, kinds: ["user"] }]);
  });

  it("R5 a buffer ending at turnDone is not working and has no new rev without a commit", () => {
    const summary = heldSummary([arrival(turnDone, { seq: null, rev: 5 })]);
    expect(summary.rev).toBeNull();
    expect(summary.turns).toEqual([]);
    expect(summary.working).toBe(false);
    expect(summary.flushable).toBe(true);
  });

  it("R6 an item at or below appliedSeq is skipped", () => {
    expect(route(arrival(assistantText("x"), { seq: 5 }), ctx())).toBe("skip");
    expect(route(arrival(assistantText("x"), { seq: 4, author: SELF }), ctx())).toBe("skip");
    expect(route(arrival(assistantText("x"), { seq: 4, author: null }), ctx({ selfId: null }))).toBe("skip");
  });

  it("R7 a page overlapping live-applied items does not duplicate them", () => {
    let view = reduceEvent(initialSessionView("running"), userMessage("one"), 1);
    view = reduceEvent(view, assistantText("two"), 2);
    const merged = mergeEntries(view, 2, {
      entries: [entry(1, userMessage("one")), entry(2, assistantText("two")), entry(3, assistantText("three"))],
      nextCursor: after(3),
    });
    expect(merged.view.items.map((it) => ("text" in it ? it.text : it.kind))).toEqual(["one", "two", "three"]);
    expect(merged.appliedSeq).toBe(3);
  });

  it("R8 out-of-order entries apply sorted by seq and existing items stay untouched", () => {
    const view = reduceEvent(initialSessionView("running"), userMessage("first"), 1);
    const existing = view.items[0];
    const merged = mergeEntries(view, 1, {
      entries: [entry(4, assistantText("c")), entry(2, userMessage("a")), entry(3, assistantText("b"))],
      nextCursor: after(4),
    });
    expect(merged.view.items[0]).toBe(existing);
    expect(merged.view.items.slice(1).map((it) => ("text" in it ? it.text : ""))).toEqual(["a", "b", "c"]);
    expect(merged.view.items.slice(1).map((it) => it.seq)).toEqual([2, 3, 4]);
  });

  it("R9 an empty page changes neither the view nor appliedSeq", () => {
    const view = reduceEvent(initialSessionView("running"), userMessage("x"), 3);
    const merged = mergeEntries(view, 3, { entries: [], nextCursor: after(7) });
    expect(merged.view).toBe(view);
    expect(merged.appliedSeq).toBe(3);
    expect(merged.firstNewSeq).toBeNull();
    expect(merged.revisions).toBe(0);
    expect(merged.reset).toBe(false);
  });

  it("R10 the optimistic own user item is not duplicated by its echo", () => {
    const view = addUserMessage(initialSessionView("running"), "mine", null, [], 500, null, "nonce-1");
    const merged = mergeEntries(view, 4, {
      entries: [entry(5, userMessage("mine", { clientMessageId: "nonce-1", author: SELF })), entry(6, assistantText("reply"))],
      nextCursor: after(6),
    });
    expect(merged.view.items.filter((it) => it.kind === "user")).toHaveLength(1);
    expect(merged.view.items.map((it) => it.kind)).toEqual(["user", "assistant"]);
    expect(merged.appliedSeq).toBe(6);
  });

  it("R11 firstNewSeq and revisions match the merged entries", () => {
    const merged = mergeEntries(initialSessionView("running"), 4, {
      entries: [
        entry(3, assistantText("old")),
        entry(5, userMessage("q", { author: OTHER })),
        entry(6, thinking("hmm")),
        entry(7, toolUse("u1")),
        entry(8, toolResult("u1")),
        entry(9, assistantText("a")),
        entry(10, turnDone),
      ],
      nextCursor: after(10),
    });
    expect(merged.firstNewSeq).toBe(5);
    expect(merged.revisions).toBe(3);
    expect(merged.appliedSeq).toBe(10);
    const user = merged.view.items.find((it) => it.kind === "user");
    expect(user).toMatchObject({ seq: 5, author: OTHER, createdAt: 1_000 });
  });

  it("R12 with no identity loaded nothing is held", () => {
    const none = ctx({ selfId: null });
    expect(route(arrival(userMessage("hi")), none)).toBe("apply");
    expect(route(arrival(assistantText("x")), none)).toBe("apply");
    expect(route(arrival({ kind: "partialText", text: "t", parentId: null }, { seq: null }), none)).toBe("apply");
    expect(route(arrival(toolUse("u1")), none)).toBe("apply");
  });

  it("R13 another author's user message during an own turn applies", () => {
    expect(route(arrival(userMessage("mid")), ctx({ ownTurnStreaming: true }))).toBe("apply");
    expect(route(arrival(userMessage("hi")), ctx())).toBe("hold");
    for (const event of [assistantText("a"), thinking("t"), toolUse("u"), toolResult("u")]) {
      expect(route(arrival(event), ctx())).toBe("hold");
    }
    expect(route(arrival(userMessage("hi"), { author: null }), ctx())).toBe("apply");
  });

  it("R14 a mid-turn user message from another author keeps the own turn's author", () => {
    const mid = arrival(userMessage("mid"));
    expect(turnAuthorAfter("self", mid, SELF.clientId, true)).toBe("self");
    expect(turnAuthorAfter("self", arrival(assistantText("own"), { author: SELF }), SELF.clientId, true)).toBe("self");
    expect(turnAuthorAfter("self", arrival(assistantText("theirs")), SELF.clientId, false)).toBe("other");
    expect(turnAuthorAfter(null, mid, SELF.clientId, false)).toBe("other");
    expect(turnAuthorAfter(null, arrival(userMessage("mine"), { author: SELF }), SELF.clientId, false)).toBe("self");
    expect(turnAuthorAfter("other", arrival(turnDone, { author: null }), SELF.clientId, true)).toBe("other");
  });

  it("R14 an author-less user message that opens a turn clears the author", () => {
    expect(turnAuthorAfter("self", arrival(userMessage("legacy"), { author: null }), SELF.clientId, false)).toBeNull();
  });

  it("R15 an own item behind held items flushes them first", () => {
    expect(route(arrival(assistantText("mine"), { author: SELF }), ctx({ heldCount: 2 }))).toBe("flush");
    expect(route(arrival(turnDone, { seq: null, author: SELF }), ctx({ heldCount: 1 }))).toBe("flush");
    expect(route(arrival(assistantText("mine"), { author: SELF }), ctx())).toBe("apply");
  });

  it("R16 a rewound item resets whatever its seq", () => {
    expect(route(arrival(rewound, { seq: 2 }), ctx({ appliedSeq: 9 }))).toBe("reset");
    expect(route(arrival(rewound, { seq: 20, author: null }), ctx())).toBe("reset");
    expect(route(arrival(rewound, { seq: 3 }), ctx({ selfId: null, heldCount: 4 }))).toBe("reset");
  });

  it("R17 a page with a rewound entry reports reset instead of merging", () => {
    const view = reduceEvent(initialSessionView("running"), userMessage("kept"), 1);
    for (const seq of [1, 6]) {
      const merged = mergeEntries(view, 4, {
        entries: [entry(5, assistantText("a")), entry(seq, rewound), entry(7, assistantText("b"))],
        nextCursor: after(7),
      });
      expect(merged.reset).toBe(true);
      expect(merged.view).toBe(view);
      expect(merged.appliedSeq).toBe(4);
      expect(merged.cursor).toEqual(after(7));
    }
  });

  it("R18 the cursor is the page's nextCursor even when trailing rows did not decode", () => {
    const merged = mergeEntries(initialSessionView("running"), 0, {
      entries: [entry(1, userMessage("x"))],
      nextCursor: after(9),
    });
    expect(merged.cursor).toEqual(after(9));
    expect(merged.appliedSeq).toBe(1);
  });

  it("a null entry seq counts as applied", () => {
    const view: SessionView = initialSessionView("running");
    const merged = mergeEntries(view, 4, { entries: [entry(null, assistantText("?"))], nextCursor: after(4) });
    expect(merged.view).toBe(view);
    expect(merged.appliedSeq).toBe(4);
  });
});

describe("manual pull: another device's turns wait for Pull", () => {
  const noticeText = (state: SyncState) =>
    renderToStaticMarkup(
      createElement(SyncComposerNotice, {
        notice: syncNotice(state, false, false),
        onPull: () => {},
        onSendAnyway: () => {},
        onLoad: () => {},
        onDismiss: () => {},
      }),
    ).replace(/<[^>]+>/g, " ");

  it("M1 a foreign revision on an idle, visible, draftless view waits instead of pulling", () => {
    const { state, effects } = run(synced(), remote());
    expect(effects).not.toContain("pull");
    expect(state.fetching).toBe(false);
    expect(state.phase).toBe("behind");
    expect(state.pendingTurns).toEqual([{ rev: 6, kinds: ["user"] }]);
    expect(canSend(state)).toBe(false);
  });

  it("M2 a foreign prompt or turn end is held even with an empty buffer", () => {
    for (const event of [permissionAsk, questionAsk, turnDone, exited]) {
      expect(route(arrival(event, { seq: 11 }), ctx())).toBe("hold");
      expect(route(arrival(event, { seq: null }), ctx())).toBe("hold");
    }
  });

  it("M3 the behind notice names the turns and the device, with a Pull button", () => {
    const behind = run(synced(), remote(), remote({ rev: 7, turns: [{ rev: 7, kinds: ["assistant"] }] })).state;
    const text = noticeText(behind);
    expect(text).toContain("2 new turns from jons-macbook-pro");
    expect(text).toContain("Pull");
    expect(text).not.toContain("pulls when you're idle");
    expect(noticeText(run(synced(), remote()).state)).toContain("1 new turn from jons-macbook-pro");
  });

  it("M4 an explicit pull fetches", () => {
    const behind = run(synced(), remote()).state;
    const { state, effects } = run(behind, { kind: "pull" });
    expect(effects).toEqual(["pull"]);
    expect(state.phase).toBe("syncing");
    const done = run(state, fetched({ rev: 6, device: OTHER.device }));
    expect(done.state.localRev).toBe(6);
    expect(done.state.phase).toBe("inSync");
    expect(done.effects).toContain("flashSynced");
  });

  it("M5 an own revision on a detached view still pulls silently", () => {
    const { state, effects } = run(synced(), remote({ own: true, device: SELF.device }));
    expect(effects).toEqual(["pull"]);
    expect(state.pendingTurns).toEqual([]);
    expect(syncNotice(state, false, false)).toBeNull();
  });

  it("M6 a foreign gap with a draft shows the guard and blocks Send", () => {
    const { state, effects } = run(synced(), remote(), { kind: "composer", draft: true });
    expect(state.phase).toBe("guard");
    expect(effects).not.toContain("pull");
    expect(canSend(state)).toBe(false);
    expect(syncNotice(state, false, false)).toMatchObject({ kind: "guard", count: 1, device: OTHER.device });
  });

  it("M7 held items with no new revision wait for Pull and block Send", () => {
    const { state, effects } = run(synced(), remote({ rev: 5, turns: [], flushable: true }));
    expect(effects).toEqual([]);
    expect(state.phase).toBe("behind");
    expect(canSend(state)).toBe(false);
    expect(noticeText(state)).toContain("New from jons-macbook-pro");
    const drafting = run(state, { kind: "composer", draft: true }).state;
    expect(drafting.phase).toBe("guard");
  });
});
