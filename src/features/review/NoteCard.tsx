import { Icon } from "@/components/Icon";
import { Button, cx } from "@/components/ui";
import type { ReviewNote } from "@/lib/bindings";
import { formatRelativeTime } from "@/lib/format";
import { useResolveReviewNote } from "@/lib/queries";

import { NOTE_KIND_LABEL } from "./threads";

const KIND_TAG: Record<ReviewNote["kind"], string> = {
  note: "border-edge text-ink-2",
  needs_your_call: "border-warn/40 bg-warn/10 text-warn-ink",
  outside_plan_scope: "border-info/40 bg-info/10 text-info",
};

export function KindTag({ kind }: { kind: ReviewNote["kind"] }) {
  return (
    <span className={cx("inline-flex h-5 shrink-0 items-center whitespace-nowrap rounded-control border px-2 text-meta font-semibold normal-case", KIND_TAG[kind])}>
      {NOTE_KIND_LABEL[kind]}
    </span>
  );
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

export function NoteCard({ note, workstreamId }: { note: ReviewNote; workstreamId: string }) {
  const resolve = useResolveReviewNote();
  return (
    <div
      className={cx(
        "space-y-1.5 rounded-card border border-edge bg-panel px-3 py-2.5",
        note.resolved && "opacity-60",
      )}
    >
      <div className="flex items-center gap-2">
        <Icon name="note" size="xs" className="text-ink-dim" />
        <KindTag kind={note.kind} />
        <span className="text-meta text-ink-4">{formatRelativeTime(note.createdAt)}</span>
        <span className="flex-1" />
        <Button
          size="sm"
          aria-pressed={note.resolved}
          busy={resolve.isPending}
          onClick={() => resolve.mutate({ workstreamId, noteId: note.id, resolved: !note.resolved })}
        >
          <Icon name="check" size="xs" />
          {note.resolved ? "Resolved" : "Resolve"}
        </Button>
      </div>
      <div className="md selectable whitespace-pre-wrap break-words text-body leading-prose text-ink">
        <InlineCode text={note.body} />
      </div>
      {note.task && <div className="mt-2 truncate text-ui-sm text-ink-dim">Plan task · {note.task}</div>}
    </div>
  );
}
