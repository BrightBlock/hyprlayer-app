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

export function docQueryFromSearch(search: LibrarySearch): ThoughtQuery {
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
