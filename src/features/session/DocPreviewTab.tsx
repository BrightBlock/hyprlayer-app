import { useQuery } from "@tanstack/react-query";

import { commands } from "../../lib/bindings";
import { unwrap } from "../../lib/ipc";
import { ErrorNote, Spinner } from "../../components/ui";
import { DocIcon } from "../../components/icons";
import { FrontmatterCard } from "../library/FrontmatterCard";
import { MarkdownPreview } from "../library/MarkdownPreview";
import { splitFrontmatter } from "../library/text";

/**
 * Read-only preview of an agent-created research/plan doc, shown in its own
 * agent-strip tab so the user can review it without leaving the project (the
 * doc id is its absolute path). Retries briefly: a freshly-written doc may not
 * be indexed for a moment after the agent's `Write`.
 */
export function DocPreviewTab({ path, title }: { path: string; title: string }) {
  const doc = useQuery({
    queryKey: ["doc", path],
    queryFn: () => unwrap(commands.getDoc(path)),
    enabled: path.length > 0,
    retry: 4,
    retryDelay: (attempt) => Math.min(1500, 300 * 2 ** attempt),
  });

  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-5">
      <div className="mx-auto w-4/5">
        <div className="mb-4 flex items-center gap-2 border-b border-edge pb-3">
          <DocIcon size={15} />
          <span className="truncate text-sm font-medium text-ink" title={path}>
            {doc.data?.meta.relPath ?? title}
          </span>
        </div>
        {doc.isLoading ? (
          <div className="flex items-center gap-2 text-sm text-ink-dim">
            <Spinner size={14} />
            Loading document…
          </div>
        ) : doc.isError ? (
          <ErrorNote error={doc.error} />
        ) : (
          <div className="space-y-4">
            <FrontmatterCard meta={doc.data!.meta} />
            <MarkdownPreview body={splitFrontmatter(doc.data!.content).body} />
          </div>
        )}
      </div>
    </div>
  );
}
