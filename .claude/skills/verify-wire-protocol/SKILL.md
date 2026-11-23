---
name: verify-wire-protocol
description: Model-check the client–server wire protocol's stream credit (Hello.stream_window, Credit), stream cancel, and runtime-event gap counting (Event.seq, EventsMissed) with TLA+. Use before merging any change to hyprlayer-proto's ControlFrame, client.rs's StreamReceiver/open_stream/data/skip_to/events_missed, streams.rs's StreamRegistry/spawn_stream/StreamTx, outbound.rs's push_event/note_missed/next_event, or lib.rs's Cancel/Credit/StreamRequest handling, and whenever a stream is reported stalled, a cancelled stream keeps sending, or the app re-reads too often or misses a change after a gap.
---

# Verify the wire protocol

Two TLA+ models, each checked by TLC over every interleaving of its steps, up
to the bounds in its `.cfg`:

- `StreamCredit.tla` covers one data-plane stream on one connection. It models
  the server's stream open (`reserve`, the written `StreamResponse`,
  `spawn_stream`), the forwarder's credit (`StreamTx::ready`, `Ready::push`),
  `Credit` and `Cancel` as the server's read loop handles them, and the
  client's `open_stream`, `Loop::data`, `StreamReceiver::recv` (credit batches)
  and `StreamReceiver::drop` (cancel).
- `EventGaps.tla` covers the runtime-event lane of one connection: the
  server's `push_event`, `missed`, `note_missed` and the writer's
  `next_event`, and the client's `Loop::event`, `skip_to`, `events_missed` and
  its skip of events it cannot decode. It checks the arithmetic as TLA+
  integers. This repo has no test file for it; the user declined generated-input tests.

## The rules

| Rule | In one sentence | The harm when it breaks |
|---|---|---|
| `StreamCredit.NoStall` | While a stream is open and you keep reading it, the server always has credit once everything sent has been read. | A terminal or session stream freezes for good with nothing in flight. |
| `StreamCredit.CreditBalance` | While a stream is open and read, the server's credit plus every unreturned byte (on the wire, queued in the client, owed in a batch, or in a `Credit` on its way back) equals the window. | Credit drifts per frame; the stream slows down, then stalls, or the window grows without bound. |
| `StreamCredit.CancelLeaksNoCredit` | Once the server has read a `Cancel`, the stream has no registry entry holding credit and no forwarder waiting for credit. At most a frame already being pushed goes out. | A cancelled stream keeps forwarding to nobody, or its credit and task stay in the registry until the connection closes. |
| `StreamCredit.CancelReleasesInbox` | Once you have dropped a stream and the server can send nothing more for it, the client holds no inbox for it. | One `Inbox` stays in the client's `stream_senders` per finished stream, for the life of the connection. |
| `EventGaps.GapsCountedOnce` | With `FLOW_CONTROL` on both sides, every seq the server numbered is counted exactly once on the client (read, skipped on purpose, or missed), and `events_missed` equals the seqs counted missed. | The app re-reads after gaps that never happened, or misses a change because a gap went uncounted. |
| `EventGaps.GapsCountedOnceWithoutFlowControl` | The same, for every seq below the client's `next_seq`, when the server sends no gap markers. | As above, with an older server. |
| `*.TypeOK` | The variables stay in their domains. | None, it only guards the model. |

## Run it

From the repo root:

```sh
.claude/skills/verify-wire-protocol/check.sh
```

It needs `java` (11+) and `mise`. It installs TLC 1.7.4 through mise, pinned by
checksum (set `TLA2TOOLS=/path/tla2tools.jar` to skip mise), and runs TLC once
per rule with `WORKERS` threads (default 4; this machine is shared). It prints
`PASS`/`FAIL` per rule, compares the list with `expected.txt`, and exits
non-zero when they differ. TLC's deadlock check is on: each model's `Done`
action marks its real end states, so any other state with no step out shows as
`ERROR`. About 6 s in all: each model has about 15,000 distinct states.
`OUT=dir` keeps the logs. Each rule's TLC output is in
`$OUT/<Model>.<Rule>/tlc.out`.

**Read every result with the pstack `/statechart` skill.** Give it the spec,
its `.cfg`, and the view module beside it (`StreamCredit-view.ts`,
`EventGaps-view.ts`), which labels the states and steps in the user's words.
Add a label there for any new action. It prints each broken rule as its
shortest path in plain words. Use it for the report to the user, not as the
verdict: `check.sh` gives the verdict.

## Known findings (the baseline in `expected.txt`)

Two rules fail on the code at the sync commit. Both are product bugs, not
modeling errors: each path below was walked step by step against the code,
though neither was replayed in a running server.

1. **`CancelReleasesInbox`: a finished stream's `Inbox` is never removed from the client's `stream_senders`.**
   `Loop::data` (client.rs) removes a stream's entry only when a data frame
   for it arrives and `inbox.tx.send` fails because the receiver is gone, or
   when the queue passes `STREAM_QUEUE_LIMIT`. `StreamReceiver::drop` sends
   `Cancel` and drops the receiver, but it cannot reach `stream_senders`, and
   the server sends no end-of-stream frame. So when no frame for the stream
   arrives after the drop, the entry stays until `read_loop` ends. Shortest
   path: the server answers with the stream id, `spawn_stream` starts the
   forwarder, the terminal closes and `run_stream` calls `registry.remove`,
   the `StreamResponse` arrives and `open_stream` registers the inbox, you drop
   the stream. Nothing more arrives, and the inbox is held. A `Cancel` that
   the server reads while the stream is still `Reserved` ends the same way.
   Each entry is one `UnboundedSender` and one `Arc<AtomicUsize>`. tokio drains
   the channel when the receiver drops, so no frames are retained. The cost is
   one map entry per finished stream on a long-lived connection: every stream
   that gets no frame after you drop it adds one.
2. **`GapsCountedOnceWithoutFlowControl`: an undecodable event swallows the gap in front of it.**
   In `read_loop`'s undecodable-event arm ("Skipped on purpose, so not
   missed"), `routes.skip_to(seq)` advances `next_seq` and drops the count it
   returns. That count includes any seqs before `seq` that never arrived, and
   they are then never added to `missed`. Shortest path: the server drops seq
   0 (`note_missed`), `next_event` discards the `Missed` item because the
   client did not announce `FLOW_CONTROL`, seq 1 is queued and written, and the
   client cannot decode it. `events_missed()` stays 0, though seq 0 was lost.
   With `FLOW_CONTROL` on both sides, the server's `EventsMissed` reaches the
   client ahead of seq 1, so the gap is counted first. That is why
   `GapsCountedOnce` passes. `RpcClient::connect` announces `Features::ALL` and
   the server always lists `flow_control`, so this path needs a server that
   does not send gap markers: an older one, or a client built with
   `connect_announcing` without `flow_control` (only e2e tests do that).

When one of these is fixed in the code, change the model to match, run
`check.sh`, and change that rule's line in `expected.txt` to `PASS`.

## Does a change touch the verified code?

Run this from the repo root:

```sh
git diff --stat 601908aa27afa5f93bb51973341b562268da28fb -- \
  crates/hyprlayer-proto/src/lib.rs \
  crates/hyprlayer-proto/src/frame.rs \
  crates/hyprlayer-server/src/client.rs \
  crates/hyprlayer-server/src/streams.rs \
  crates/hyprlayer-server/src/outbound.rs \
  crates/hyprlayer-server/src/lib.rs \
  crates/hyprlayer-server/src/events.rs
```

Anything listed means: read the diff against the tables below. A change to an
event's or stream's content, the stream sources (`forward_terminal`,
`forward_session`, replay), the link window (`link.rs`, `Received`), part
frames, heartbeats, or request slots does not move the models. A change to
what credit a frame takes or returns, the client's batch size, when
`Credit` or `Cancel` is sent or applied, the `Reserved`/`Open` registry
entries, how seqs are numbered or merged into a `Missed` item, or how the
client counts `lost` does move them.

`StreamCredit.tla`:

| Model action | Code (at the sync commit) |
|---|---|
| `Batch` | `credit_batch`, client.rs:179 (`stream_window / 4`, at least 1) |
| `Window`, `Announced` | `STREAM_WINDOW`, proto lib.rs:96; the server's hello, server lib.rs:650; `spawn_stream`'s `Credit::new(window)`, streams.rs:1203 |
| `SrvRespond` | `streams.reserve` then `push_frame_written`, server lib.rs:870–875 |
| `SrvSpawn` | `spawn_stream`, streams.rs:1203 (the `Reserved { cancelled: true }` check at 1227, under the registry lock) |
| `SrvReady` | `StreamTx::ready`, streams.rs:501 (`credit.available()`: at least 1 byte) |
| `SrvPush` | `Ready::push`, streams.rs:550 (`credit.take(frame.len() - STREAM_FRAME_HEADER)`) |
| `SrvSourceEnds` | a forwarder returning, then `registry.remove`, streams.rs:1270 |
| `SrvReadCredit` | `Control::Credit` → `StreamRegistry::grant`, server lib.rs:911, streams.rs:462 (only an `Open` entry) |
| `SrvReadCancel` | `Control::Cancel` → `StreamRegistry::abort`, server lib.rs:907, streams.rs:450 |
| `CliReadResp` | `read_loop`'s `StreamResponse` arm → `open_stream`, client.rs:506 |
| `CliReadData` | `Loop::data`, client.rs:529 |
| `CliRecv` | `StreamReceiver::recv`, client.rs:712 (`owed`, `batch`) |
| `CliDrop` | `impl Drop for StreamReceiver`, client.rs:729 |

`EventGaps.tla`:

| Model action | Code (at the sync commit) |
|---|---|
| `SrvQueue`, `SrvDrop` | `Outbound::push_event`, outbound.rs:480 (no room while a gap is open, past `EVENT_QUEUE_FRAMES`, or past `EVENT_QUEUE_BYTES`; an unserializable event is dropped too) |
| `SrvLag` | `Outbound::missed`, outbound.rs:526, from `clients.missed(n)` on a lagged broadcast, events.rs:462 |
| `NoteMissed` | `Lanes::note_missed`, outbound.rs:358 (merges into a trailing `Missed`) |
| `SrvWrite` | `Lanes::next_event`, outbound.rs:310 (a `Missed` item is dropped without `flow_control`, line 329) |
| `CliEventKept`, `CliEventFull` | `Loop::event` → `skip_to`, client.rs:478/487 (`try_send` `Full` adds one) |
| `CliEventSkipped` | `read_loop`'s undecodable-event arm, client.rs:615 |
| `CliMissed` | `Loop::events_missed`, client.rs:493 |

Granularity: an action is one step only where the code makes it one. That
means a section under a mutex (`StreamRegistry`'s map, `Lanes`, the client's
`stream_senders`), one frame read by a read loop, or one synchronous stretch of
a task between awaits. The forwarder's `ready` and `push` are separate steps
because the source's `recv` await sits between them. An abort that lands
while the forwarder is between them lets it push once more (`aborting`).

## Bounds and what the models leave out

- `StreamCredit`: one stream, `Window = 8`, `Announced = 8` (so `Batch = 2`),
  frame sizes 1, 3 and 9 (one larger than the window, which the code allows:
  credit "may go negative by one frame"), at most 6 frames. `EventGaps`: seqs
  0–5, `EVENT_QUEUE_FRAMES` = 2, a lag of 1 or 2, with gap markers on or off.
- Left out of `StreamCredit`: the link window and `Received` (the server's
  read loop spawns every request and never blocks, and the client's never
  waits on a consumer, so a `Credit` or `Cancel` is always read; the model
  delivers each direction in order, eventually), the writer's per-stream queue
  (`STREAM_QUEUE_BYTES`, refilled as frames are written), the client's
  `STREAM_QUEUE_LIMIT` cancel (unreachable while the server honours credit),
  connection close and `abort_all`, a client without `FLOW_CONTROL` (no credit
  window and no cancel on drop), and several streams sharing a connection.
- Left out of `EventGaps`: superseded revisions (`superseded` rewrites a
  queued event in place under its own seq, which no count sees), `seq: None`
  from servers that predate it, u64 overflow at `u64::MAX`, and timing
  (`GapPacer`).
- A PASS covers these bounds only.

## Real bug or modeling error?

For each FAIL, take its trace (`/statechart` gives it in words, or
`$OUT/<Model>.<Rule>/tlc.out`) and walk it action by action through the code
in the tables above. If every step is possible in the code, it is a bug.
Report it with the path in the code's words. Do not weaken the rule or the
model to make it pass. If a step is impossible in the code (the code holds a
lock the model does not, or orders two writes the model lets race), the model
is wrong: fix the action, re-run, and say what you changed.

## When a change is intended

1. Change the actions that mirror the changed code, keeping one action per
   indivisible code step, and update the tables.
2. Prove the rule you touched can still fail. Break that code path in the model
   on purpose and see `check.sh` report it, then undo the break. Breaks that
   were each confirmed to fail their rule:
   - `NoStall`: `Announced = 40` in `StreamCredit.cfg`, a window announced
     larger than the one kept, so a batch outgrows the window.
   - `CreditBalance`: `Charge(n) == n + 1`, the server counting a byte of
     header the client does not.
   - `CancelLeaksNoCredit`: `IF FALSE` for `SrvSpawn`'s
     `entry = "reservedCancelled"` test, or an abort that leaves a pushing
     forwarder `pushing`.
   - `GapsCountedOnce`: `next' = next` in `NoteMissed` (seqs reused after a
     drop), or `SrvWrite` dropping `Missed` items even with gap markers on.
3. Run `check.sh`. Update `expected.txt` only for a rule whose verdict the
   change was meant to move, and say so in the PR.
4. Move the sync commit (the hash in the `git diff` command above and below)
   forward only after you have read the covered code at the new commit and
   confirmed the models describe it.

Last confirmed against: `601908aa27afa5f93bb51973341b562268da28fb`.
