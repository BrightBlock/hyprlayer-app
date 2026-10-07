import type { ThoughtQuery } from "@/lib/bindings";
import { MIN_DOC_TEXT_QUERY } from "@/lib/queries";
import { asLibraryBox, boxQuery } from "./libraryBoxes";
import { listFiltersActive, type LibrarySearch } from "./librarySearch";

export const DOC_PAGE_SIZE = 50;

export function splitParam(value: string | undefined): string[] {
  return value
    ? value
        .split(",")
        .map((part) => part.trim())
        .filter(Boolean)
    : [];
}

function listFilters(search: LibrarySearch): ThoughtQuery {
  return {
    types: splitParam(search.type),
    statuses: splitParam(search.status),
    scopes: splitParam(search.scope),
    authors: splitParam(search.author),
    tags: splitParam(search.tag),
    tagMode: "or",
  };
}

export function docQueryFromSearch(search: LibrarySearch): ThoughtQuery {
  const hasText = (search.q?.length ?? 0) >= MIN_DOC_TEXT_QUERY;
  return {
    ...listFilters(search),
    text: hasText ? (search.q ?? null) : null,
    projects: splitParam(search.project),
    sort: hasText ? "relevance" : (search.sort ?? "mtime"),
    limit: DOC_PAGE_SIZE,
    // The box wins on its own dimension.
    ...boxQuery(asLibraryBox(search.box)),
  };
}

// The nav's counts apply the list's filters so each box counts the list it opens.
export function countFiltersFromSearch(search: LibrarySearch): ThoughtQuery | null {
  return listFiltersActive(search) ? listFilters(search) : null;
}
