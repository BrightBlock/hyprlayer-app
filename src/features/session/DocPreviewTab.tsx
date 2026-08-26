import { useQuery } from "@tanstack/react-query";

import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { ErrorNote, Spinner } from "@/components/ui";
import { DocIcon } from "@/components/icons";
import { FrontmatterCard } from "@/features/library/FrontmatterCard";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";
import { splitFrontmatter } from "@/features/library/text";

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
      <div className="mx-auto w-[90%]">
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
