import type { ThoughtQuery } from "@/lib/bindings";
import { MIN_DOC_TEXT_QUERY } from "@/lib/queries";
import type { LibrarySearch } from "./librarySearch";
import { DOC_PAGE_SIZE } from "./constants";

export function splitParam(value: string | undefined): string[] {
  return value
    ? value
        .split(",")
        .map((part) => part.trim())
        .filter(Boolean)
    : [];
}

/**
 * The library query, built from the URL filter params. The selected-doc/body-mode
 * params (`doc`/`mode`) are view state, not query inputs, so they're ignored here.
 */
export function docQueryFromSearch(search: LibrarySearch): ThoughtQuery {
  // Below MIN_DOC_TEXT_QUERY a text query is a full-corpus disk scan, so a
  // short query narrows nothing (the list stays unfiltered by text) rather
  // than firing one — the text itself stays in the box either way, since
  // `search.q` (the URL param) is untouched here.
  const hasText = (search.q?.length ?? 0) >= MIN_DOC_TEXT_QUERY;
  return {
    text: hasText ? (search.q ?? null) : null,
    types: splitParam(search.type),
    statuses: splitParam(search.status),
    projects: splitParam(search.project),
    scopes: splitParam(search.scope),
    authors: splitParam(search.author),
    tags: splitParam(search.tag),
    tagMode: "or",
    sort: hasText ? "relevance" : "date",
    limit: DOC_PAGE_SIZE,
    offset: 0,
  };
}
