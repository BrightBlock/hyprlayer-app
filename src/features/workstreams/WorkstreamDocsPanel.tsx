import { useMemo, useState } from "react";

import { Badge, Button, ErrorNote, Select, Spinner, TextInput } from "../../components/ui";
import { DOC_TYPES, STATUS_TONE } from "../library/constants";
import type { DocMeta, ThoughtQuery } from "../../lib/bindings";
import {
  useAttachWorkstreamDoc,
  useDetachWorkstreamDoc,
  useDocs,
  useWorkstreamDocs,
} from "../../lib/queries";

/** Attachment role derived from the doc's type. */
export function roleForDoc(doc: DocMeta): string {
  return doc.fm.type === "research" || doc.fm.type === "plan" ? doc.fm.type : "other";
}

function titleFor(doc: DocMeta): string {
  return doc.fm.title ?? doc.relPath.split("/").pop() ?? doc.relPath;
}

type Props = {
  workstreamId: string;
  /** Project mapped name; docs are scoped to it. */
  mappedName: string | null;
  onOpenDoc: (id: string) => void;
};

/**
 * The pinned Docs tab: the project's thoughts docs, inline — browse, open,
 * and attach/detach them as context for the active workstream without
 * leaving the workstream view.
 */
export function WorkstreamDocsPanel({ workstreamId, mappedName, onOpenDoc }: Props) {
  const [text, setText] = useState("");
  const [type, setType] = useState("");

  const query: ThoughtQuery = useMemo(
    () => ({
      text: text.trim() ? text.trim() : null,
      types: type ? [type] : [],
      statuses: [],
      projects: mappedName ? [mappedName] : [],
      scopes: [],
      authors: [],
      tags: [],
      tagMode: "or",
      dateFrom: null,
      dateTo: null,
      sort: text.trim() ? "relevance" : "date",
      limit: 50,
      offset: 0,
    }),
    [mappedName, text, type],
  );
  const docs = useDocs(query, Boolean(mappedName));
  const attached = useWorkstreamDocs(workstreamId);
  const attach = useAttachWorkstreamDoc(workstreamId);
  const detach = useDetachWorkstreamDoc(workstreamId);

  const attachedPaths = new Set((attached.data ?? []).map((d) => d.docPath));

  return (
    <div className="mx-auto max-w-3xl space-y-3 p-5">
      <div className="flex items-center gap-2">
        <TextInput
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Search project docs…"
        />
        <Select
          value={type}
          onChange={(e) => setType(e.target.value)}
          className="w-36"
          aria-label="Filter by type"
        >
          <option value="">All types</option>
          {DOC_TYPES.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </Select>
      </div>

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
        <div className="overflow-hidden rounded-lg border border-edge bg-panel">
          {docs.data.items.map((doc) => {
            const isAttached = attachedPaths.has(doc.id);
            return (
              <div
                key={doc.id}
                className="flex items-start justify-between gap-3 border-b border-edge/60 px-4 py-3 last:border-b-0 hover:bg-panel-2/70"
              >
                <button
                  type="button"
                  onClick={() => onOpenDoc(doc.id)}
                  className="min-w-0 flex-1 text-left"
                >
                  <p className="truncate text-sm font-medium text-ink">{titleFor(doc)}</p>
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
                      isAttached
                        ? detach.mutate(doc.id)
                        : attach.mutate({ docPath: doc.id, role: roleForDoc(doc) })
                    }
                    busy={attach.isPending || detach.isPending}
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
      {(attach.isError || detach.isError) && (
        <ErrorNote error={attach.isError ? attach.error : detach.error} />
      )}
    </div>
  );
}
