import { useState, type ReactNode } from "react";
import { openPath } from "@tauri-apps/plugin-opener";

import { DocIcon, ImageIcon, XIcon } from "@/components/icons";
import { attachmentSrc, type Attachment } from "./attachments";

/**
 * A pin card for one prompt attachment, an image thumbnail or a file glyph plus
 * the filename. Used both in the composer (with a remove button) and inline in
 * the conversation, so the full path the agent received never shows as text.
 */
/** The 22px leading square, an icon glyph for files (and the fallback when an
 *  image thumbnail can't load). */
function Glyph({ children }: { children: ReactNode }) {
  return (
    <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded bg-accent/15 text-accent">
      {children}
    </span>
  );
}

function ImageThumb({ path, name }: { path: string; name: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <Glyph><ImageIcon size={12} /></Glyph>;
  return (
    <img
      src={attachmentSrc(path)}
      alt={name}
      onError={() => setFailed(true)}
      className="h-[22px] w-[22px] shrink-0 rounded object-cover"
    />
  );
}

export function AttachmentCard({
  attachment,
  onRemove,
}: {
  attachment: Attachment;
  /** When set, shows a remove button (composer use); omit for read-only cards. */
  onRemove?: () => void;
}) {
  const { path, name, kind } = attachment;
  const open = () => void openPath(path).catch(() => {});

  return (
    <span
      className="inline-flex max-w-56 items-center gap-1 rounded-md border border-accent/40 bg-accent/10 p-0.5 pr-1 text-xs text-ink"
      title={name}
    >
      {kind === "image" ? (
        <ImageThumb path={path} name={name} />
      ) : (
        <Glyph>
          <DocIcon size={12} />
        </Glyph>
      )}
      <button
        type="button"
        onClick={open}
        className="min-w-0 truncate px-0.5 text-left font-medium hover:text-accent"
        title={`Open ${name}`}
      >
        {name}
      </button>
      {onRemove && (
        <button
          type="button"
          onClick={onRemove}
          className="inline-flex h-4 w-4 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-accent/20 hover:text-ink"
          title="Remove attachment"
          aria-label={`Remove ${name}`}
        >
          <XIcon size={9} strokeWidth={2.4} />
        </button>
      )}
    </span>
  );
}

/** A wrapping row of attachment cards. Renders nothing when empty. */
export function AttachmentCards({
  attachments,
  onRemove,
  className,
}: {
  attachments: Attachment[] | undefined;
  onRemove?: (path: string) => void;
  className?: string;
}) {
  if (!attachments || attachments.length === 0) return null;
  return (
    <div className={`flex flex-wrap items-center gap-1.5 ${className ?? ""}`}>
      {attachments.map((att) => (
        <AttachmentCard
          key={att.path}
          attachment={att}
          onRemove={onRemove ? () => onRemove(att.path) : undefined}
        />
      ))}
    </div>
  );
}
