import { memo, type MouseEvent } from "react";

import { DocStatusPill } from "@/components/DocStatusPill";
import { Icon } from "@/components/Icon";
import { ContextMenu, cx, MenuItem, RowSpine } from "@/components/ui";
import type { DocMeta } from "@/lib/bindings";
import { HARNESS_SHORT_LABEL } from "@/lib/domain/harnessing";
import { LIST_ROW } from "@/lib/listNav";
import { useMarkDocRead, useMarkDocUnread } from "@/lib/queries";
import { DocAvatar } from "./DocAvatar";
import { docTypeIcon } from "./docTypeIcon";
import { docTitle } from "./text";

const CHIP = "inline-flex h-5 shrink-0 items-center gap-1 rounded-control bg-panel-2 px-1.5 text-ui-sm text-ink-dim";

export type MailRowMenuTarget = { id: string; unread: boolean; x: number; y: number };

function fromLabel(author: string | null, agent: string | null): string | null {
  if (author && agent) return `${author} with ${agent}`;
  return agent ?? author;
}

function ticketLabel(ticket: string): string {
  if (!/^https?:\/\//i.test(ticket)) return ticket;
  try {
    return new URL(ticket).pathname.split("/").filter(Boolean).pop() ?? ticket;
  } catch {
    return ticket;
  }
}

export const MailRow = memo(function MailRow({
  doc,
  selected,
  time,
  project,
  onOpen,
  onMenu,
}: {
  doc: DocMeta;
  selected: boolean;
  time: string;
  project: string | null;
  onOpen: (id: string) => void;
  onMenu: (target: MailRowMenuTarget) => void;
}) {
  const { fm } = doc;
  const unread = doc.unread ?? false;
  const agent = doc.agent ?? null;
  const author = fm.author ?? null;
  const from = fromLabel(author, agent ? HARNESS_SHORT_LABEL[agent.harness] : null);
  const related = fm.related?.length ?? 0;

  return (
    <button
      type="button"
      {...LIST_ROW}
      onClick={() => onOpen(doc.id)}
      onContextMenu={(e: MouseEvent) => {
        e.preventDefault();
        onMenu({ id: doc.id, unread, x: e.clientX, y: e.clientY });
      }}
      aria-current={selected ? "true" : undefined}
      className={cx(
        "relative flex w-full gap-2.5 px-3.5 py-2.5 text-left",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/60",
        selected ? "bg-accent-soft" : "hover:bg-edge/50",
      )}
    >
      {selected && <RowSpine />}
      <DocAvatar author={author} agent={agent} type={fm.type ?? null} />
      <span className="block min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          {unread && (
            <span role="img" aria-label="Unread" className="h-[7px] w-[7px] shrink-0 rounded-full bg-info" />
          )}
          <span className={cx("min-w-0 truncate text-ui font-semibold", from ? "text-ink" : "text-ink-4")}>
            {from ?? "Unknown author"}
          </span>
          {project && <span className="shrink-0 whitespace-nowrap text-meta text-ink-4">· {project}</span>}
          <span className="ml-auto shrink-0 pl-1 text-meta text-ink-4">{time}</span>
        </span>
        <span
          className={cx("mt-0.5 block truncate text-body text-ink", unread ? "font-bold" : "font-medium")}
        >
          {docTitle(doc)}
        </span>
        {doc.excerpt && (
          <span className="mt-0.5 line-clamp-2 text-ui-sm text-ink-dim">{doc.excerpt}</span>
        )}
        <span className="mt-1.5 flex gap-1.5 overflow-hidden">
          {fm.type && (
            <span className={CHIP}>
              <Icon name={docTypeIcon(fm.type)} size="xs" />
              {fm.type.toLowerCase()}
            </span>
          )}
          {fm.status && <DocStatusPill status={fm.status} />}
          {fm.ticket && <span className={cx(CHIP, "text-meta")}>{ticketLabel(fm.ticket)}</span>}
          {related > 0 && (
            <span className={CHIP}>
              <Icon name="linkDoc" size="xs" label="Related" />
              {related}
            </span>
          )}
        </span>
      </span>
    </button>
  );
});

// One menu for the whole list, so the rows carry no mutation hooks.
export function MailRowMenu({ target, onClose }: { target: MailRowMenuTarget; onClose: () => void }) {
  const { mutate: markRead } = useMarkDocRead();
  const { mutate: markUnread } = useMarkDocUnread();
  return (
    <ContextMenu position={{ x: target.x, y: target.y }} onClose={onClose}>
      <MenuItem
        onClick={() => {
          onClose();
          if (target.unread) markRead(target.id);
          else markUnread(target.id);
        }}
      >
        {target.unread ? "Mark as read" : "Mark as unread"}
      </MenuItem>
    </ContextMenu>
  );
}
