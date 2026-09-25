import { useState, type ReactNode } from "react";
import { openPath } from "@tauri-apps/plugin-opener";

import { Icon } from "@/components/Icon";
import { useAttachmentSrc, useCanOpenAttachment, type Attachment } from "./attachments";

function Glyph({ children }: { children: ReactNode }) {
  return (
    <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded bg-accent/15 text-accent">
      {children}
    </span>
  );
}

function ImageThumb({ path, name }: { path: string; name: string }) {
  const [failed, setFailed] = useState(false);
  const { src, error } = useAttachmentSrc(path);
  // A fetch that failed is not a fetch still running: say which, on hover,
  // rather than sitting on the loading glyph forever.
  if (failed || error || !src) {
    return (
      <span title={error ?? (failed ? `Could not display ${name}` : undefined)}>
        <Glyph>
          <Icon name="image" size="xs" />
        </Glyph>
      </span>
    );
  }
  return (
    <img
      src={src}
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
  onRemove?: () => void;
}) {
  const { path, name, kind } = attachment;
  // `openPath` resolves on *this* machine, so a remote server's path opens
  // nothing and fails silently. Offering a dead button is worse than saying so.
  const canOpen = useCanOpenAttachment();
  const open = () => void openPath(path).catch(() => {});

  return (
    <span
      className="inline-flex max-w-56 items-center gap-1 rounded-md border border-accent/40 bg-accent/10 p-0.5 pr-1 text-ui text-ink"
      title={name}
    >
      {kind === "image" ? (
        <ImageThumb path={path} name={name} />
      ) : (
        <Glyph>
          <Icon name="doc" size="xs" />
        </Glyph>
      )}
      <button
        type="button"
        onClick={canOpen ? open : undefined}
        disabled={!canOpen}
        className="min-w-0 truncate px-0.5 text-left font-medium enabled:hover:text-accent disabled:cursor-default"
        title={canOpen ? `Open ${name}` : `${name} — on the server, so it cannot be opened here`}
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
          <Icon name="close" size="xs" />
        </button>
      )}
    </span>
  );
}

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
