import { memo, useMemo } from "react";

import { Icon } from "@/components/Icon";
import { rowClass } from "@/components/ui";
import { currentAppLocation, openReview } from "@/lib/appNav";
import { plural } from "@/lib/format";
import { findWorkstreamForNotify } from "@/lib/notifications";
import type { ReviewCardProps } from "@/features/session/reviewCardSlot";

import { KindTag } from "./NoteCard";
import { parseReviewMessage, type ReviewMessageItem } from "./reviewMessage";

// Implement transcripts carry no plan-card context, so fall back to the open workstream.
function routeWorkstreamId(): string | null {
  const item = currentAppLocation()?.search.item;
  return typeof item === "string" ? item : null;
}

function open(workstreamId: string | null, item: ReviewMessageItem) {
  const id = workstreamId ?? routeWorkstreamId();
  if (!id) return;
  const target = findWorkstreamForNotify(id);
  if (!target) return;
  openReview({ projectPath: target.projectPath, workstreamId: id, file: item.path, line: item.line });
}

export const ReviewCard = memo(function ReviewCard({ text, workstreamId, fallback }: ReviewCardProps) {
  const message = useMemo(() => parseReviewMessage(text), [text]);
  if (!message) return fallback;
  return (
    <div className="space-y-2 text-ui">
      <div className="flex items-center gap-1.5 font-medium text-ink">
        <Icon name="review" size="xs" className="text-accent" />
        <span className="min-w-0 truncate">
          Review · Changes requested · {message.viewed} / {message.files} viewed ·{" "}
          {plural(message.count, "comment")}
        </span>
      </div>
      {message.summary && (
        <p className="selectable whitespace-pre-wrap break-words text-body text-ink">{message.summary}</p>
      )}
      {message.items.length > 0 && (
        <ul className="border-l border-edge pl-1.5">
          {message.items.map((item, i) => (
            <li key={i}>
              <button
                type="button"
                onClick={() => open(workstreamId, item)}
                title={item.note ? `${item.note.body}\n\n${item.body}` : item.body}
                className={rowClass({ tier: "compact", className: "flex w-full gap-2 px-1.5 text-left" })}
              >
                <code className="shrink-0 font-mono text-meta text-ink-2">
                  {item.path}:{item.line}
                </code>
                {item.note && <KindTag kind={item.note.kind} />}
                <span className="min-w-0 flex-1 truncate text-ink">{item.body.split("\n")[0]}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
});
