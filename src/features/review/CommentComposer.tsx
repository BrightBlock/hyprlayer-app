import { useEffect, useRef, useState } from "react";

import { Button, ErrorNote, TextArea } from "@/components/ui";
import type { AddReviewCommentRequest } from "@/lib/bindings";
import { useAddReviewComment } from "@/lib/queries";

export function CommentComposer({
  request,
  agentLabel,
  onDone,
}: {
  request: Omit<AddReviewCommentRequest, "body">;
  agentLabel: string;
  onDone: () => void;
}) {
  const [body, setBody] = useState("");
  const add = useAddReviewComment();
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    ref.current?.focus({ preventScroll: true });
  }, []);

  const submit = () => {
    const text = body.trim();
    if (!text || add.isPending) return;
    add.mutate({ ...request, body: text }, { onSuccess: onDone });
  };

  return (
    <div className="space-y-2 rounded-card border border-edge bg-panel p-2.5">
      <TextArea
        ref={ref}
        rows={3}
        className="px-3 py-2"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder={`Leave a comment for ${agentLabel}…`}
        aria-label={`Leave a comment for ${agentLabel}`}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            e.stopPropagation();
            submit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onDone();
          }
        }}
      />
      {add.error != null && <ErrorNote error={add.error} onDismiss={() => add.reset()} />}
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-ui-sm text-ink-dim">Held until you submit the review</span>
        <Button size="sm" onClick={onDone}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" disabled={!body.trim()} busy={add.isPending} onClick={submit}>
          Add to review
        </Button>
      </div>
    </div>
  );
}
