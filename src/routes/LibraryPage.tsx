import { useNavigate, useSearch } from "@tanstack/react-router";
import { useMemo, useState } from "react";

import { Button, Spinner } from "../components/ui";
import { CreateDocDialog } from "../features/library/CreateDocDialog";
import { DocCountSummary, DocFilterFields, docQueryFromSearch } from "../features/library/DocFilters";
import { DocList } from "../features/library/DocList";
import { useBootstrap, useDocs } from "../lib/queries";
import { useCommand } from "../lib/keybindings/registry";
import type { LibrarySearch } from "../router";

export function LibraryPage() {
  const search = useSearch({ from: "/library" });
  const navigate = useNavigate({ from: "/library" });
  const bootstrap = useBootstrap();
  const [createOpen, setCreateOpen] = useState(false);

  // "New Document" — registered only while the Library is open (its context).
  useCommand("doc.new", () => setCreateOpen(true));

  const query = useMemo(() => docQueryFromSearch(search), [search]);
  const docs = useDocs(query);

  const update = (patch: Partial<LibrarySearch>) =>
    void navigate({
      search: (prev) => ({ ...prev, ...patch }),
      replace: true,
    });

  return (
    <div className="flex h-full min-h-0">
      <aside className="w-64 shrink-0 overflow-y-auto border-r border-edge bg-panel p-4">
        <div className="flex items-center justify-between gap-2">
          <h1 className="text-base font-semibold">Library</h1>
          <Button variant="primary" onClick={() => setCreateOpen(true)}>
            New
          </Button>
        </div>

        <DocFilterFields
          search={search}
          onUpdate={update}
          projectOptions={bootstrap.data?.projects ?? []}
        />
      </aside>

      <section className="min-w-0 flex-1 overflow-y-auto">
        <div className="sticky top-0 z-10 border-b border-edge bg-surface/95 px-6 py-4 backdrop-blur">
          <div className="flex items-center justify-between gap-4">
            <div>
              <DocCountSummary result={docs.data} />
            </div>
            {docs.isFetching && <Spinner />}
          </div>
        </div>

        <div className="p-6">
          <DocList
            result={docs.data}
            isLoading={docs.isLoading}
            isError={docs.isError}
            error={docs.error}
            emptyText="No documents match the current filters."
          />
        </div>
      </section>

      <CreateDocDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        initialProjectName={search.project}
      />
    </div>
  );
}
