import { Link } from "@tanstack/react-router";

import { Badge, Card, ErrorNote, Spinner } from "../../components/ui";
import type { DocMeta, QueryResult } from "../../lib/bindings";
import { STATUS_TONE } from "./constants";
import { docTitle } from "./text";

export const DOC_PAGE_SIZE = 50;

function subtitleFor(meta: DocMeta): string {
  const parts = [meta.project ?? meta.fm.project, meta.fm.author, meta.fm.date].filter(Boolean);
  return parts.length > 0 ? parts.join(" · ") : meta.relPath;
}

export function DocRow({ doc }: { doc: DocMeta }) {
  return (
    <Link
      to="/library/doc"
      search={{ id: doc.id, mode: "preview" }}
      className="block border-b border-edge/60 px-5 py-3 hover:bg-panel-2/70"
    >
      <div className="flex min-w-0 items-start justify-between gap-4">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium text-ink">{docTitle(doc)}</p>
          <p className="selectable mt-0.5 truncate font-mono text-xs text-ink-dim">{doc.relPath}</p>
          <p className="mt-1 truncate text-xs text-ink-dim">{subtitleFor(doc)}</p>
        </div>
        <div className="flex shrink-0 flex-wrap justify-end gap-1">
          {doc.fm.type && <Badge tone="accent">{doc.fm.type}</Badge>}
          {doc.fm.status && <Badge tone={STATUS_TONE[doc.fm.status] ?? "dim"}>{doc.fm.status}</Badge>}
          <Badge tone="dim">{doc.scope}</Badge>
        </div>
      </div>
      {(doc.fm.tags?.length ?? 0) > 0 && (
        <div className="mt-2 flex flex-wrap gap-1">
          {doc.fm.tags!.slice(0, 8).map((tag) => (
            <span key={tag} className="rounded bg-panel px-1.5 py-0.5 text-[11px] text-ink-dim">
              #{tag}
            </span>
          ))}
        </div>
      )}
    </Link>
  );
}

type DocListProps = {
  result?: QueryResult;
  isLoading: boolean;
  isError: boolean;
  error: unknown;
  emptyText: string;
};

export function DocList({ result, isLoading, isError, error, emptyText }: DocListProps) {
  return (
    <>
      {isError && <ErrorNote error={error} />}
      {isLoading && (
        <div className="flex justify-center p-12">
          <Spinner />
        </div>
      )}
      {result && result.items.length === 0 && (
        <Card className="text-sm text-ink-dim">{emptyText}</Card>
      )}
      {result && result.items.length > 0 && (
        <div className="overflow-hidden rounded-lg border border-edge bg-panel">
          {result.items.map((doc) => (
            <DocRow key={doc.id} doc={doc} />
          ))}
        </div>
      )}
    </>
  );
}
