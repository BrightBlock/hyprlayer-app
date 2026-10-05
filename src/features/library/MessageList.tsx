import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Icon } from "@/components/Icon";
import { MenuPanel, OptionMenuList } from "@/components/OptionMenuList";
import { EmptyState, ErrorNote, IconButton, Popover, Spinner, TextInput } from "@/components/ui";
import type { DocMeta, Project } from "@/lib/bindings";
import { plural } from "@/lib/format";
import { useDocCounts, useDocPages } from "@/lib/queries";
import { docQueryFromSearch } from "./DocFilters";
import { asLibraryBox, LIBRARY_BOXES } from "./libraryBoxes";
import { libraryFiltersActive, type LibrarySearch, type LibrarySort } from "./librarySearch";
import { ListFilterPopover, listFiltersActive } from "./ListFilterPopover";
import { dateGroup, mailTime, parseDocDate, type DateGroup } from "./mailDates";
import { MailRow, MailRowMenu, type MailRowMenuTarget } from "./MailRow";

const SORT_ITEMS: { key: LibrarySort; label: string }[] = [
  { key: "mtime", label: "Modified" },
  { key: "date", label: "Date" },
  { key: "title", label: "Title" },
];

function rowStamp(doc: DocMeta, sort: LibrarySort): { ms: number; dateOnly: boolean } | null {
  const date = sort === "date" ? parseDocDate(doc.fm.date) : null;
  if (date) return date;
  return doc.mtimeMs === null ? null : { ms: doc.mtimeMs, dateOnly: false };
}

export function MessageList({
  search,
  onUpdate,
  projects,
  projectName,
}: {
  search: LibrarySearch;
  onUpdate: (patch: Partial<LibrarySearch>) => void;
  projects: Project[];
  projectName: string | undefined;
}) {
  const query = docQueryFromSearch(search);
  // By content: `search` also changes on every row click.
  const queryKey = JSON.stringify(query);
  const docs = useDocPages(query);
  const authors = useDocCounts(search.project).data?.authors ?? [];

  const scrollerRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const filterRef = useRef<HTMLButtonElement>(null);
  const sortRef = useRef<HTMLButtonElement>(null);
  const [filterOpen, setFilterOpen] = useState(false);
  const [sortOpen, setSortOpen] = useState(false);
  const [menu, setMenu] = useState<MailRowMenuTarget | null>(null);

  const updateRef = useRef(onUpdate);
  updateRef.current = onUpdate;
  const openDoc = useCallback((id: string) => updateRef.current({ doc: id }), []);

  useEffect(() => {
    if (scrollerRef.current) scrollerRef.current.scrollTop = 0;
  }, [queryKey]);

  const { hasNextPage, isFetchingNextPage, isPlaceholderData, fetchNextPage } = docs;
  useEffect(() => {
    const el = sentinelRef.current;
    const root = scrollerRef.current;
    if (!el || !root || !hasNextPage || isFetchingNextPage || isPlaceholderData) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) void fetchNextPage();
      },
      { root, rootMargin: "200px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [hasNextPage, isFetchingNextPage, isPlaceholderData, fetchNextPage]);

  const pages = docs.data?.pages;
  const first = pages?.[0];
  // Offsets shift if the index changes between pages, which can repeat a row.
  const items = useMemo(() => {
    const seen = new Set<string>();
    return (pages ?? []).flatMap((p) => p.items).filter((d) => !seen.has(d.id) && seen.add(d.id));
  }, [pages]);

  const sort = search.sort ?? "mtime";
  const grouped = query.sort === "mtime" || query.sort === "date";
  const projectLabel = useMemo(() => new Map(projects.map((p) => [p.thoughtsProject, p.mappedName])), [projects]);
  const now = new Date();
  let lastGroup: DateGroup | null = null;

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-panel">
      <div className="flex h-13 shrink-0 items-center gap-2 border-b border-edge px-3.5">
        <div className="min-w-0 flex-1">
          <p className="truncate text-title text-ink">{LIBRARY_BOXES[asLibraryBox(search.box)].label}</p>
          {first && (
            <p className="truncate text-meta text-ink-4">
              {first.total} across {plural(first.projects ?? 0, "project")}
              {(first.unread ?? 0) > 0 && ` · ${first.unread} new`}
            </p>
          )}
        </div>
        <IconButton
          ref={filterRef}
          onClick={() => setFilterOpen((o) => !o)}
          active={filterOpen || listFiltersActive(search)}
          aria-haspopup="dialog"
          aria-expanded={filterOpen}
          aria-label="Filter"
          title="Filter"
        >
          <Icon name="filter" size="sm" />
        </IconButton>
        <IconButton
          ref={sortRef}
          onClick={() => setSortOpen((o) => !o)}
          active={sortOpen}
          aria-haspopup="menu"
          aria-expanded={sortOpen}
          aria-label="Sort"
          title="Sort"
        >
          <Icon name="sort" size="sm" />
        </IconButton>
      </div>

      <div className="shrink-0 border-b border-edge-2 px-3.5 py-2">
        <div className="relative">
          <span className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-4">
            <Icon name="search" size="xs" />
          </span>
          <TextInput
            value={search.q ?? ""}
            onChange={(e) => onUpdate({ q: e.target.value || undefined })}
            placeholder="Search titles, bodies, tickets"
            aria-label="Search the library"
            className="h-7 bg-surface py-0 pl-7 text-ui placeholder:text-ink-4"
          />
        </div>
      </div>

      <div ref={scrollerRef} className="min-h-0 flex-1 overflow-y-auto">
        {docs.isError && (
          <div className="p-3">
            <ErrorNote error={docs.error} />
          </div>
        )}
        {docs.isLoading && (
          <div className="flex justify-center p-8">
            <Spinner />
          </div>
        )}
        {first &&
          items.length === 0 &&
          (projectName && !libraryFiltersActive(search) ? (
            <p className="px-4 py-6 font-mono text-meta text-ink-dim">no thoughts in {projectName} yet</p>
          ) : (
            <EmptyState title="Nothing here" />
          ))}
        <div className="divide-y divide-edge/60">
          {items.map((doc) => {
            const stamp = rowStamp(doc, sort);
            const group = grouped ? (stamp ? dateGroup(stamp.ms, now) : "Earlier") : null;
            const header = group !== null && group !== lastGroup;
            lastGroup = group;
            const project = doc.project ?? doc.fm.project ?? null;
            return (
              <Fragment key={doc.id}>
                {header && (
                  <p className="bg-panel px-3.5 pb-1.5 pt-2.5 text-ui-sm font-semibold text-ink-dim">{group}</p>
                )}
                <MailRow
                  doc={doc}
                  selected={search.doc === doc.id}
                  time={stamp ? mailTime(stamp.ms, now, stamp.dateOnly) : ""}
                  project={project && (projectLabel.get(project) ?? project)}
                  onOpen={openDoc}
                  onMenu={setMenu}
                />
              </Fragment>
            );
          })}
        </div>
        <div ref={sentinelRef} aria-hidden="true" />
        {isFetchingNextPage && (
          <div className="flex justify-center p-4">
            <Spinner size="sm" />
          </div>
        )}
      </div>

      <ListFilterPopover
        open={filterOpen}
        onClose={() => setFilterOpen(false)}
        anchorRef={filterRef}
        search={search}
        onUpdate={onUpdate}
        authors={authors}
      />
      <Popover
        open={sortOpen}
        onClose={() => setSortOpen(false)}
        anchorRef={sortRef}
        align="end"
        className="min-w-40"
      >
        <MenuPanel aria-label="Sort">
          <OptionMenuList
            items={SORT_ITEMS}
            activeKey={sort}
            onSelect={(key) => {
              onUpdate({ sort: key === "mtime" ? undefined : (key as LibrarySort) });
              setSortOpen(false);
            }}
          />
        </MenuPanel>
      </Popover>
      {menu && <MailRowMenu target={menu} onClose={() => setMenu(null)} />}
    </div>
  );
}
