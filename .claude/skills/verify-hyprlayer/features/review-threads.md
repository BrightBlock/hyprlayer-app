# Review notes, threads and verdicts

Inside the review workspace, the agent's review notes render inline at their line with a kind tag. A note can be replied to and resolved. The reviewer adds line comments, which are held as `Pending` until submitted. Threads take replies, edits, deletes and resolve. Resolved or outdated items fold into a bar with `Show`. `Submit review` offers three verdicts. `Comment` saves the comments, `Approve` marks the review approved, and `Request changes` sends every unsent comment to the implement agent as one message. That message appears in the chat as a review card. A request that didn't reach the agent shows `Not delivered · Resend`.

## Sub-features

- `rt-note` covers a note card: the kind tag (`Note`, `Outside plan scope`), `Outdated`, `Plan task · …`, `Reply` and `Resolve`/`Resolved`. <!-- src: src/features/review/threads.ts:6 --> <!-- src: src/features/review/NoteCard.tsx:59 --> <!-- src: src/features/review/NoteCard.tsx:88 -->
- `rt-comment` adds a line comment through the gutter `+`, then `Add to review`. <!-- src: src/features/review/commentGutter.ts:50 --> <!-- src: src/features/review/CommentComposer.tsx:59 -->
- `rt-thread` covers a thread: `Reply`, `Resolve`, `Edit comment`/`Save`, `Delete comment` and the state badge (`Pending`, `Saved`, `Sending`, `Sent to <agent>`, `Addressed`). <!-- src: src/features/review/CommentThread.tsx:217 --> <!-- src: src/features/review/CommentThread.tsx:19 --> <!-- src: src/features/review/CommentThread.tsx:21 --> <!-- src: src/features/review/CommentThread.tsx:23 -->
- `rt-fold` folds resolved and outdated items into `Resolved · Outdated · … · Show`. <!-- src: src/features/review/CommentThread.tsx:114 --> <!-- src: src/features/review/CommentThread.tsx:141 -->
- `rt-submit` covers the `Submit review` popover: `Summary`, the `Verdict` radios and the primary button label. <!-- src: src/features/review/SubmitReviewPopover.tsx:146 --> <!-- src: src/features/review/SubmitReviewPopover.tsx:154 -->
- `rt-deliver` covers the request-changes round trip: the toast `N comments sent to <agent>`, the review card in the chat and `Not delivered · Resend`. <!-- src: src/features/review/useReviewDelivery.ts:30 --> <!-- src: src/features/review/ReviewCard.tsx:35 --> <!-- src: src/features/review/ReviewHeader.tsx:48 -->

## How to get to it (user POV)

- Open the review workspace ([review-workspace.md](./review-workspace.md)) and select a file with notes or threads. <!-- src: src/features/review/FileTree.tsx:197 -->
- Hover a diff line and click the `+` in the gutter to comment on it. <!-- src: src/features/review/commentGutter.ts:270 -->
- Click `Submit review` in the workspace header. The Ship check's `Submit first` also opens it. <!-- src: src/features/review/ReviewHeader.tsx:166 --> <!-- src: src/features/review/ShipCheckDialog.tsx:56 -->

## Driving it with the MCP bridge

Preconditions:

- **Seed.** Use the `review_lifecycle` sandbox from [review-workspace.md](./review-workspace.md), which is isolated. Save `get_review` as `00-review.json` first. A freshly kept sandbox's comments are all `sent`, so no `Edit comment`/`Delete comment` shows until you add one.
- **No model turns.** `Request changes` delivers to the sandbox's stub `claude`, so it costs no tokens. <!-- src: crates/hyprlayer-e2e/tests/review_lifecycle.rs:133 --> On a real profile, the same click starts a real implement turn. <!-- src: src/features/session/sessionStore.ts:2165 -->
- **Nothing here touches GitHub.**
- **No undo for verdicts.** Every verdict appends a round, and rounds can't be deleted. Treat the sandbox as disposable, and re-keep it for a clean rerun.

Steps:

- **Note.** Select `src/new.rs`. Its note card shows `Outside plan scope` and `Plan task · Phase 2: new`. <!-- src: src/features/review/threads.ts:7 --> <!-- src: src/features/review/NoteCard.tsx:88 --> Folded items show a button ending in `Show`; click it to expand. <!-- src: src/features/review/CommentThread.tsx:141 -->
- **Reply to a note.** This is the real-click route to a composer. Click the note's `Reply`, which only shows while the note has no replies. <!-- src: src/features/review/NoteCard.tsx:63 --> <!-- src: src/features/review/DiffPane.tsx:194 --> Then `webview_keyboard type` into `textarea[aria-label^="Leave a comment for "]` <!-- src: src/features/review/CommentComposer.tsx:39 --> and click `Add to review`. <!-- src: src/features/review/CommentComposer.tsx:59 --> The reply shows `Pending`, and `Submit review` gains a count badge. <!-- src: src/features/review/CommentThread.tsx:17 --> <!-- src: src/features/review/ReviewHeader.tsx:167 --> Read back: `get_review` has a new comment with `state` `pending` and `replyToNoteId` set, and that note is now `resolved`, because a reply resolves its note. Restoring the note through its `Resolve` toggle doesn't remove the reply. <!-- src: crates/hyprlayer-core/src/store/reviews.rs:278 -->
- **Line comment.** The gutter `+` only exists while a `mousemove` hovers a line, and a `mousedown` on `.cm-review-gutter` opens the composer. SKILL.md only says bridge clicks send no pointer events; whether a `webview_interact` click sends `mousemove` or `mousedown` is unverified. <!-- src: src/features/review/commentGutter.ts:246 --> <!-- src: src/features/review/commentGutter.ts:276 --> To drive it, use `webview_execute_js` to dispatch a `mousemove` on the modified editor's `.cm-content` at a line's `clientY`, then a `mousedown` on the matching `.cm-review-gutter` element. Report it as synthetic; it doesn't prove the hover path.
- **Edit and delete.** Only `Pending` and `Saved` comments have `[aria-label="Edit comment"]` and `[aria-label="Delete comment"]`. Sent ones don't. <!-- src: src/features/review/CommentThread.tsx:12 --> <!-- src: src/features/review/CommentThread.tsx:59 --> <!-- src: src/features/review/CommentThread.tsx:67 --> Edit opens a textarea with the same label, which saves with `Save`. <!-- src: src/features/review/CommentThread.tsx:84 --> <!-- src: src/features/review/CommentThread.tsx:103 --> Delete has no confirm: the comment is gone from `get_review` at once.
- **Resolve.** A thread's or note's `Resolve` button flips to `Resolved` with `aria-pressed="true"`, and the item folds. <!-- src: src/features/review/CommentThread.tsx:232 --> <!-- src: src/features/review/NoteCard.tsx:77 --> Read back `resolved: true`, then click again to restore it.
- **Submit: Comment.** Click `Submit review`. <!-- src: src/features/review/ReviewHeader.tsx:166 --> `[role="dialog"][aria-label="Submit review"]` opens. <!-- src: src/features/review/SubmitReviewPopover.tsx:53 --> Click the `Comment` radio inside `[role="radiogroup"][aria-label="Verdict"]`. <!-- src: src/features/review/SubmitReviewPopover.tsx:154 --> <!-- src: src/lib/useRovingRadio.ts:45 --> The primary button reads `Save comments`. Click it, and a `Comments saved` toast shows. <!-- src: src/features/review/SubmitReviewPopover.tsx:19 --> <!-- src: src/features/review/SubmitReviewPopover.tsx:128 --> Read back: the last round has `verdict` `comment`, and the pending comment is now `saved`.
- **Submit: Request changes.** Add a comment, then pick `Request changes`. The primary reads `Send 1 comment to <agent>`. <!-- src: src/features/review/SubmitReviewPopover.tsx:15 --> With no unsent comments, it's disabled until `textarea[aria-label="Summary"]` has text. <!-- src: src/features/review/SubmitReviewPopover.tsx:115 --> <!-- src: src/features/review/SubmitReviewPopover.tsx:146 --> After sending:
  - The toast reads `1 comment sent to <agent>`. <!-- src: src/features/review/useReviewDelivery.ts:30 -->
  - The header chip reads `Changes requested`. <!-- src: src/features/review/ReviewHeader.tsx:55 -->
  - The implement chat shows a card `Review · Changes requested · V / T viewed · N comments`, whose rows reopen the review at `path:line`. <!-- src: src/features/review/ReviewCard.tsx:35 -->
  - Read back: the last round has `verdict` `request_changes` and `delivered: true`, the comments are `sent`, and `status` is `changes_requested`. `$SB/harness.log` holds the delivered message (`Address each comment, then stop. …`).
- **Submit: Approve.** It's the default verdict when nothing is unsent. <!-- src: src/features/review/SubmitReviewPopover.tsx:92 --> The toast reads `Approved`, `Nothing is pushed until you click Ship`, and the chip reads `Approved`. <!-- src: src/features/review/SubmitReviewPopover.tsx:127 --> Read back: `status` is `approved`.
- **Not delivered.** If delivery throws, a `Not delivered` toast offers `Resend`, and the header chip becomes `Not delivered · Resend`, which disables `Submit review`. <!-- src: src/features/review/useReviewDelivery.ts:23 --> <!-- src: src/features/review/ReviewHeader.tsx:48 --> <!-- src: src/features/review/ReviewHeader.tsx:162 -->
- **Proof.** Save screenshots of the note, the composer, each popover state and the chat card, plus `get_review` after every submit.

## Gotchas

- Enter doesn't submit under the bridge. Click `Add to review`, `Save` and the popover's primary button. ⌘Enter is wired, but keyboard chords are unreliable over the bridge. <!-- src: src/features/review/CommentComposer.tsx:41 -->
- `Request changes` is disabled while the implement agent is busy (`<agent> is still working`), and the default verdict then becomes `Comment`. <!-- src: src/features/review/SubmitReviewPopover.tsx:103 -->
- The agent label is the implement session's harness, `Claude Code` in the sandbox (its stub), and `the agent` only when the workstream has no implement session. Match composer labels by prefix. <!-- src: src/lib/domain/harnessing.ts:55 -->
- With no live implement session, delivery starts a new `general` session titled `Review`. In the sandbox that's the stub; on a real profile it's a real turn. <!-- src: src/features/session/sessionStore.ts:2177 -->
- Agent notes come from `.hyprlayer/review/<workstream-id>.jsonl` in the worktree and are ingested on the next `get_review`. Append a JSON line there to add a case without rerunning the scenario. <!-- src: crates/hyprlayer-core/src/pipeline.rs:241 -->
- An outdated item sits at the nearest surviving line, and in unified view a base-side thread lists above the diff. Don't read its position as a bug. <!-- src: src/features/review/DiffPane.tsx:142 -->
