import type { ThoughtQuery } from "../../lib/bindings";
import type { LibrarySearch } from "../../router";
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
  return {
    text: search.q ?? null,
    types: splitParam(search.type),
    statuses: splitParam(search.status),
    projects: splitParam(search.project),
    scopes: splitParam(search.scope),
    authors: splitParam(search.author),
    tags: splitParam(search.tag),
    tagMode: "or",
    sort: search.q ? "relevance" : "date",
    limit: DOC_PAGE_SIZE,
    offset: 0,
  };
}
