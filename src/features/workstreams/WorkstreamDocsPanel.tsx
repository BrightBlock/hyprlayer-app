import { useMemo, useState } from "react";

import { Badge, Button, DropdownSelect, ErrorNote, Spinner, TextInput } from "@/components/ui";
import { DOC_TYPES, STATUS_TONE } from "@/features/library/constants";
import { docTitle } from "@/features/library/text";
import type { DocMeta, ThoughtQuery } from "@/lib/bindings";
import { useDocs, MIN_DOC_TEXT_QUERY } from "@/lib/queries";

/** Attachment role derived from the doc's type. */
export function roleForDoc(doc: DocMeta): string {
  return doc.fm.type === "research" || doc.fm.type === "plan" ? doc.fm.type : "other";
}

type Props = {
  /** The project's thoughts key (the doc-index project id); docs are scoped to it. */
  thoughtsProject: string | null;
  /** Title of the agent tab attachments will target (the last-active one). */
  targetTitle: string | null;
  /** Doc ids already attached to that tab. */
  attachedPaths: string[];
  onAttach: (id: string, role: string) => void;
  onDetach: (id: string) => void;
  onOpenDoc: (id: string) => void;
};

/**
 * The Docs view: the project's thoughts docs, inline, browse, open, and
 * attach/detach them as context for the active agent tab without leaving
 * the workstream.
 */
export function WorkstreamDocsPanel({
  thoughtsProject,
  targetTitle,
  attachedPaths,
  onAttach,
  onDetach,
  onOpenDoc,
}: Props) {
  const [text, setText] = useState("");
  const [type, setType] = useState("");

  const query: ThoughtQuery = useMemo(() => {
    const trimmed = text.trim();
    // Below MIN_DOC_TEXT_QUERY a text query is a full-corpus disk scan, so a
    // short query leaves the panel unnarrowed rather than firing one.
    const hasText = trimmed.length >= MIN_DOC_TEXT_QUERY;
    return {
      text: hasText ? trimmed : null,
      types: type ? [type] : [],
      statuses: [],
      projects: thoughtsProject ? [thoughtsProject] : [],
      scopes: [],
      authors: [],
      tags: [],
      tagMode: "or",
      dateFrom: null,
      dateTo: null,
      sort: hasText ? "relevance" : "date",
      limit: 50,
      offset: 0,
    };
  }, [thoughtsProject, text, type]);
  const docs = useDocs(query, Boolean(thoughtsProject));
  const attached = new Set(attachedPaths);

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <TextInput
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Search project docs…"
        />
        <DropdownSelect
          value={type}
          onChange={setType}
          className="w-36"
          aria-label="Filter by type"
          options={[
            { value: "", label: "All types" },
            ...DOC_TYPES.map((t) => ({ value: t, label: t })),
          ]}
        />
      </div>
      {targetTitle && (
        <p className="text-xs text-ink-dim">
          Attachments become context for <span className="font-medium text-ink">{targetTitle}</span>.
        </p>
      )}

      {docs.isError && <ErrorNote error={docs.error} />}
      {docs.isLoading && (
        <div className="flex justify-center p-10">
          <Spinner />
        </div>
      )}
      {docs.data && docs.data.items.length === 0 && (
        <p className="rounded-lg border border-edge bg-panel p-4 text-sm text-ink-dim">
          No documents yet. Research and plan docs created in this workstream's worktree will
          appear here.
        </p>
      )}
      {docs.data && docs.data.items.length > 0 && (
        <div className="max-h-[60vh] divide-y divide-edge/60 overflow-y-auto rounded-lg border border-edge bg-panel">
          {docs.data.items.map((doc) => {
            const isAttached = attached.has(doc.id);
            return (
              <div
                key={doc.id}
                className="flex items-start justify-between gap-3 px-4 py-3 hover:bg-edge/50"
              >
                <button
                  type="button"
                  onClick={() => onOpenDoc(doc.id)}
                  className="min-w-0 flex-1 text-left"
                >
                  <p className="truncate text-sm font-medium text-ink">{docTitle(doc)}</p>
                  <p className="selectable mt-0.5 truncate font-mono text-xs text-ink-dim">
                    {doc.relPath}
                  </p>
                </button>
                <div className="flex shrink-0 items-center gap-1.5">
                  {doc.fm.type && <Badge tone="accent">{doc.fm.type}</Badge>}
                  {doc.fm.status && (
                    <Badge tone={STATUS_TONE[doc.fm.status] ?? "dim"}>{doc.fm.status}</Badge>
                  )}
                  <Button
                    onClick={() =>
                      isAttached ? onDetach(doc.id) : onAttach(doc.id, roleForDoc(doc))
                    }
                    className="px-2 py-1 text-xs"
                  >
                    {isAttached ? "Detach" : "Attach"}
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
