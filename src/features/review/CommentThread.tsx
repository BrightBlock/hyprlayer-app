import { useState, type ReactNode } from "react";

import { Icon } from "@/components/Icon";
import { Badge, type BadgeTone, Button, cx, ErrorNote, IconButton, TextArea } from "@/components/ui";
import type { ReviewComment, ReviewCommentState } from "@/lib/bindings";
import { formatRelativeTime, plural } from "@/lib/format";
import { useDeleteReviewComment, useEditReviewComment, useResolveReviewComment } from "@/lib/queries";

import { CommentComposer } from "./CommentComposer";
import { type Thread, threadSize } from "./threads";

const EDITABLE: ReadonlySet<ReviewCommentState> = new Set(["pending", "saved"]);

function stateBadge(comment: ReviewComment, agentLabel: string): { label: string; tone: BadgeTone } {
  switch (comment.state) {
    case "pending":
      return { label: "Pending", tone: "dim" };
    case "saved":
      return { label: "Saved", tone: "accent" };
    case "sending":
      return { label: "Sending", tone: "warn" };
    case "sent":
      return comment.addressed ? { label: "Addressed", tone: "accent" } : { label: `Sent to ${agentLabel}`, tone: "ok" };
  }
}

function CommentBody({
  comment,
  workstreamId,
  agentLabel,
}: {
  comment: ReviewComment;
  workstreamId: string;
  agentLabel: string;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const edit = useEditReviewComment();
  const remove = useDeleteReviewComment();
  const badge = stateBadge(comment, agentLabel);
  const editable = EDITABLE.has(comment.state);

  const save = () => {
    const body = draft?.trim();
    if (!body || edit.isPending) return;
    edit.mutate({ workstreamId, commentId: comment.id, body }, { onSuccess: () => setDraft(null) });
  };

  return (
    <div className="space-y-1.5 py-2.5 first:pt-0 last:pb-0">
      <div className="flex items-center gap-2">
        <Badge tone={badge.tone}>{badge.label}</Badge>
        <span className="text-meta text-ink-4">{formatRelativeTime(comment.createdAt)}</span>
        <span className="flex-1" />
        {editable && draft === null && (
          <>
            <IconButton
              className="h-6 w-6 hover:bg-edge/60"
              title="Edit"
              aria-label="Edit comment"
              onClick={() => setDraft(comment.body)}
            >
              <Icon name="edit" size="xs" />
            </IconButton>
            <IconButton
              className="h-6 w-6 hover:bg-edge/60"
              title="Delete"
              aria-label="Delete comment"
              disabled={remove.isPending}
              onClick={() => remove.mutate({ workstreamId, commentId: comment.id })}
            >
              <Icon name="close" size="xs" />
            </IconButton>
          </>
        )}
      </div>
      {draft === null ? (
        <p className="selectable whitespace-pre-wrap break-words text-body leading-prose text-ink">{comment.body}</p>
      ) : (
        <div className="space-y-2">
          <TextArea
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            aria-label="Edit comment"
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                e.stopPropagation();
                save();
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                setDraft(null);
              }
            }}
          />
          {edit.error != null && <ErrorNote error={edit.error} onDismiss={() => edit.reset()} />}
          <div className="flex justify-end gap-2">
            <Button size="sm" onClick={() => setDraft(null)}>
              Cancel
            </Button>
            <Button size="sm" variant="primary" disabled={!draft.trim()} busy={edit.isPending} onClick={save}>
              Save
            </Button>
          </div>
        </div>
      )}
      {remove.error != null && <ErrorNote error={remove.error} onDismiss={() => remove.reset()} />}
    </div>
  );
}

function foldLabel(resolved: boolean, outdated: boolean): string {
  return [resolved && "Resolved", outdated && "Outdated"].filter(Boolean).join(" · ");
}

/** A resolved or outdated conversation folded to one row, as GitHub does. `flush` sits inside a box. */
export function FoldedBar({
  resolved,
  outdated,
  summary,
  flush = false,
  onShow,
}: {
  resolved: boolean;
  outdated: boolean;
  summary: ReactNode;
  flush?: boolean;
  onShow: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onShow}
      className={cx(
        "flex w-full items-center gap-1.5 px-3 py-1.5 text-left text-ui text-ink-dim hover:bg-edge/50",
        !flush && "rounded-card border border-edge bg-panel",
      )}
    >
      <Icon name={resolved ? "check" : "outdated"} size="xs" />
      {foldLabel(resolved, outdated)} · {summary} · <span className="text-ink-2 underline-offset-2 hover:underline">Show</span>
    </button>
  );
}

/** `embedded` drops the card, for a thread answering a note inside the note's box. */
export function CommentThread({
  thread,
  workstreamId,
  agentLabel,
  context,
  outdated = false,
  embedded = false,
}: {
  thread: Thread;
  workstreamId: string;
  agentLabel: string;
  context?: ReactNode;
  outdated?: boolean;
  embedded?: boolean;
}) {
  const [shown, setShown] = useState(false);
  const [replying, setReplying] = useState(false);
  const resolve = useResolveReviewComment();
  const { root } = thread;
  const count = plural(threadSize(thread), "comment");
  const folds = root.resolved || outdated;

  if (folds && !shown) {
    return (
      <FoldedBar
        resolved={root.resolved}
        outdated={outdated}
        summary={count}
        flush={embedded}
        onShow={() => setShown(true)}
      />
    );
  }

  return (
    <div className={cx("space-y-2 px-3 py-2.5", !embedded && "rounded-card border border-edge bg-panel")}>
      {folds && (
        <div className="flex items-center gap-1.5 text-ui text-ink-dim">
          <Icon name={root.resolved ? "check" : "outdated"} size="xs" />
          <span className="flex-1">
            {foldLabel(root.resolved, outdated)} · {count}
          </span>
          <IconButton className="h-6 w-6 hover:bg-edge/60" aria-label="Collapse thread" onClick={() => setShown(false)}>
            <Icon name="chevronDown" size="xs" />
          </IconButton>
        </div>
      )}
      {context}
      <div className="divide-y divide-edge/60">
        {[root, ...thread.replies].map((c) => (
          <CommentBody key={c.id} comment={c} workstreamId={workstreamId} agentLabel={agentLabel} />
        ))}
      </div>
      {resolve.error != null && <ErrorNote error={resolve.error} onDismiss={() => resolve.reset()} />}
      {replying ? (
        <CommentComposer
          request={{
            workstreamId,
            path: root.path,
            side: root.side,
            line: root.line,
            anchorText: root.anchorText,
            parentId: root.id,
          }}
          agentLabel={agentLabel}
          onDone={() => setReplying(false)}
        />
      ) : (
        <div className="flex items-center gap-2">
          <Button size="sm" onClick={() => setReplying(true)}>
            Reply
          </Button>
          <Button
            size="sm"
            aria-pressed={root.resolved}
            busy={resolve.isPending}
            onClick={() => {
              const resolved = !root.resolved;
              resolve.mutate(
                { workstreamId, commentId: root.id, resolved },
                { onSuccess: () => resolved && setShown(false) },
              );
            }}
          >
            <Icon name="check" size="xs" />
            {root.resolved ? "Resolved" : "Resolve"}
          </Button>
        </div>
      )}
    </div>
  );
}
