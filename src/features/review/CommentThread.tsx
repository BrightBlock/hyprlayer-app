import { useState, type ReactNode } from "react";

import { Icon } from "@/components/Icon";
import { Badge, type BadgeTone, Button, ErrorNote, IconButton, TextArea } from "@/components/ui";
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

export function CommentThread({
  thread,
  workstreamId,
  agentLabel,
  context,
}: {
  thread: Thread;
  workstreamId: string;
  agentLabel: string;
  context?: ReactNode;
}) {
  const [shown, setShown] = useState(false);
  const [replying, setReplying] = useState(false);
  const resolve = useResolveReviewComment();
  const { root } = thread;
  const count = plural(threadSize(thread), "comment");

  if (root.resolved && !shown) {
    return (
      <button
        type="button"
        onClick={() => setShown(true)}
        className="flex w-full items-center gap-1.5 rounded-card border border-edge bg-panel px-3 py-1.5 text-left text-ui text-ink-dim hover:bg-edge/50"
      >
        <Icon name="check" size="xs" />
        Resolved · {count} · <span className="text-ink-2 underline-offset-2 hover:underline">Show</span>
      </button>
    );
  }

  return (
    <div className="space-y-2 rounded-card border border-edge bg-panel px-3 py-2.5">
      {root.resolved && (
        <div className="flex items-center gap-1.5 text-ui text-ink-dim">
          <Icon name="check" size="xs" />
          <span className="flex-1">Resolved · {count}</span>
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
            onClick={() =>
              resolve.mutate(
                { workstreamId, commentId: root.id, resolved: !root.resolved },
                { onSuccess: () => setShown(false) },
              )
            }
          >
            <Icon name="check" size="xs" />
            {root.resolved ? "Resolved" : "Resolve"}
          </Button>
        </div>
      )}
    </div>
  );
}
