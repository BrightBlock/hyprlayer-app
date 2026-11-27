// The TypeScript half of the differential: `node --import ./register.mjs ts-replay.mjs [--perf <log> <items>] | <name>=<log>... --out <dir>`.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { performance } from "node:perf_hooks";

import { cancelPendingPrompts, initialSessionView, reduceEvent, resolvePermission, resolveQuestion, restoredView, settleUnwaitedAsks } from "../../src/features/session/transcript.ts";
import { closedTurnIn, mergeEntries } from "../../src/features/session/transcriptSync.ts";

const MERGE_PAGE = 25;

// Stored rows predate `quiet`; the wire always carries it (serde default false), so the client never sees it missing.
const onWire = (row) => (row.event.kind === "relaunched" && row.event.quiet === undefined ? { ...row, event: { ...row.event, quiet: false } } : row);

const readLog = (file) =>
  readFileSync(file, "utf8")
    .split("\n")
    .filter((l) => l.trim())
    .map((l) => onWire(JSON.parse(l)));

const strideOf = (events) => (events <= 500 ? 1 : Math.ceil(events / 32));

// Number formatting and key order match replay.rs's `canon`; nulls are dropped.
function canon(v) {
  if (v === null || v === undefined) return "null";
  if (Array.isArray(v)) return `[${v.map(canon).join(",")}]`;
  if (typeof v === "object") {
    const keys = Object.keys(v).filter((k) => v[k] !== null && v[k] !== undefined).sort();
    return `{${keys.map((k) => `${JSON.stringify(k)}:${canon(v[k])}`).join(",")}}`;
  }
  return typeof v === "string" ? JSON.stringify(v) : String(v);
}
const hash = (text) => createHash("sha256").update(text).digest("hex").slice(0, 16);
function snapshot(view) {
  const { items, ...rest } = view;
  return { status: canon(rest), itemCount: items.length, items: items.map((it) => hash(canon(it))) };
}

function foldLog(entries, onStep) {
  let view = initialSessionView("starting");
  entries.forEach((stored, i) => {
    view = reduceEvent(view, stored.event, stored.seq);
    const last = view.items[view.items.length - 1];
    if (stored.kind === "userMessage" && last?.kind === "user" && last.createdAt === null) {
      view = { ...view, items: [...view.items.slice(0, -1), { ...last, createdAt: stored.createdAt }] };
    }
    onStep?.(i, view);
  });
  return view;
}

function replay(entries) {
  const stride = strideOf(entries.length);
  const checkpoints = [];
  const fold = foldLog(entries, (i, view) => {
    if ((i + 1) % stride === 0 || i + 1 === entries.length) checkpoints.push({ index: i, view: snapshot(view) });
  });
  const open = new Set(
    [fold.pendingPermission, fold.pendingQuestion, ...fold.queuedQuestions].filter(Boolean).map((a) => a.requestId),
  );
  const pages = [];
  let merged = initialSessionView("starting");
  let applied = -1;
  for (let at = 0; at < entries.length; at += MERGE_PAGE) {
    const m = mergeEntries(merged, applied, { entries: entries.slice(at, at + MERGE_PAGE), nextCursor: { kind: "empty" } });
    merged = m.view;
    applied = m.appliedSeq;
    pages.push({
      revisions: m.revisions,
      firstNewSeq: m.firstNewSeq,
      appliedSeq: m.appliedSeq,
      reset: m.reset,
      closedTurn: m.closedTurn,
    });
    if (m.reset) break;
  }
  return {
    events: entries.length,
    checkpoints,
    foldItems: fold.items.map(canon),
    fold: snapshot(fold),
    restored: snapshot(restoredView(fold, false)),
    restoredLive: snapshot(restoredView(settleUnwaitedAsks(fold, open), true)),
    settledNone: snapshot(settleUnwaitedAsks(fold, new Set())),
    cancelled: snapshot(cancelPendingPrompts(fold)),
    resolvedPermission: fold.pendingPermission
      ? snapshot(resolvePermission(fold, fold.pendingPermission.requestId, "allow"))
      : null,
    resolvedQuestion: fold.pendingQuestion ? snapshot(resolveQuestion(fold, fold.pendingQuestion.requestId)) : null,
    merge: { view: snapshot(merged), pages },
    closedTurn: closedTurnIn(entries),
  };
}

const args = process.argv.slice(2);
if (args[0] === "--perf") {
  const entries = readLog(args[1]);
  const target = Number(args[2]);
  let cut = null;
  foldLog(entries, (i, view) => {
    if (cut === null && view.items.length >= target) cut = i + 1;
  });
  cut ??= entries.length;
  const started = performance.now();
  const view = foldLog(entries.slice(0, cut));
  console.log(JSON.stringify({ events: cut, items: view.items.length, ms: performance.now() - started }));
} else {
  const outAt = args.indexOf("--out");
  const out = args[outAt + 1];
  mkdirSync(out, { recursive: true });
  for (const arg of args.slice(0, outAt)) {
    const eq = arg.indexOf("=");
    writeFileSync(path.join(out, `${arg.slice(0, eq)}.json`), JSON.stringify(replay(readLog(arg.slice(eq + 1)))));
  }
}
