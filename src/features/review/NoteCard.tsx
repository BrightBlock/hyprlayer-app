import { Icon } from "@/components/Icon";
import { useState, type ReactNode } from "react";

import { Button, cx, IconButton } from "@/components/ui";
import type { ReviewNote } from "@/lib/bindings";
import { formatRelativeTime, plural } from "@/lib/format";
import { useResolveReviewNote } from "@/lib/queries";

import { FoldedBar } from "./CommentThread";
import { NOTE_KIND_LABEL } from "./threads";

const TAG = "inline-flex h-5 shrink-0 items-center whitespace-nowrap rounded-control border px-2 text-meta font-semibold normal-case";

const KIND_TAG: Record<ReviewNote["kind"], string> = {
  note: "border-edge text-ink-2",
  outside_plan_scope: "border-info/40 bg-info/10 text-info",
};

export function KindTag({ kind }: { kind: ReviewNote["kind"] }) {
  return <span className={cx(TAG, KIND_TAG[kind])}>{NOTE_KIND_LABEL[kind]}</span>;
}

// Note bodies are short markdown; only inline code is rendered.
function InlineCode({ text }: { text: string }) {
  return text
    .split(/(`[^`\n]+`)/g)
    .map((part, i) =>
      part.length > 2 && part.startsWith("`") && part.endsWith("`") ? (
        <code key={i} className="rounded bg-panel-2 px-1 py-0.5 text-[0.85em]">
          {part.slice(1, -1)}
        </code>
      ) : (
        part
      ),
    );
}

export function NoteCard({
  note,
  workstreamId,
  outdated,
  onReply,
  onResolved,
  onCollapse,
}: {
  note: ReviewNote;
  workstreamId: string;
  outdated: boolean;
  onReply?: () => void;
  onResolved?: () => void;
  onCollapse?: () => void;
}) {
  const resolve = useResolveReviewNote();
  return (
    <div className={cx("space-y-1.5 px-3 py-2.5", note.resolved && "opacity-60")}>
      <div className="flex flex-wrap items-center gap-2">
        <Icon name="note" size="xs" className="text-ink-dim" />
        <KindTag kind={note.kind} />
        {outdated && <span className={cx(TAG, "border-warn/40 bg-warn/10 text-warn-ink")}>Outdated</span>}
        <span className="text-meta text-ink-4">{formatRelativeTime(note.createdAt)}</span>
        <span className="flex-1" />
        {onReply && (
          <Button size="sm" onClick={onReply}>
            Reply
          </Button>
        )}
        <Button
          size="sm"
          aria-pressed={note.resolved}
          busy={resolve.isPending}
          onClick={() => {
            const resolved = !note.resolved;
            resolve.mutate({ workstreamId, noteId: note.id, resolved }, { onSuccess: () => resolved && onResolved?.() });
          }}
        >
          <Icon name="check" size="xs" />
          {note.resolved ? "Resolved" : "Resolve"}
        </Button>
        {onCollapse && (
          <IconButton className="h-6 w-6 hover:bg-edge/60" aria-label="Collapse conversation" onClick={onCollapse}>
            <Icon name="chevronDown" size="xs" />
          </IconButton>
        )}
      </div>
      <div className="md selectable whitespace-pre-wrap break-words text-body leading-prose text-ink">
        <InlineCode text={note.body} />
      </div>
      {note.task && <div className="mt-2 truncate text-ui-sm text-ink-dim">Plan task · {note.task}</div>}
    </div>
  );
}

/**
 * A note and its replies in one box, folded once the note is resolved or outdated. A reply
 * resolves its note, so the conversation stays open while the reviewer is still writing in it.
 */
export function NoteConversation({
  note,
  workstreamId,
  replyCount,
  outdated,
  composer,
  onReply,
  children,
}: {
  note: ReviewNote;
  workstreamId: string;
  replyCount: number;
  outdated: boolean;
  composer?: ReactNode;
  onReply?: () => void;
  children?: ReactNode;
}) {
  const [shown, setShown] = useState(false);
  if (composer && !shown) setShown(true);
  const folds = note.resolved || outdated;

  if (folds && !shown) {
    const kind = NOTE_KIND_LABEL[note.kind];
    const summary = replyCount > 0 ? `${kind} · ${plural(replyCount, "comment")}` : kind;
    return <FoldedBar resolved={note.resolved} outdated={outdated} summary={summary} onShow={() => setShown(true)} />;
  }
  return (
    <div className="divide-y divide-edge/60 overflow-hidden rounded-card border border-edge bg-panel">
      <NoteCard
        note={note}
        workstreamId={workstreamId}
        outdated={outdated}
        onReply={onReply}
        onResolved={() => setShown(false)}
        onCollapse={folds ? () => setShown(false) : undefined}
      />
      {children}
      {composer && <div className="px-3 py-2.5">{composer}</div>}
    </div>
  );
}
