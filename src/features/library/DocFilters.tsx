import type { ThoughtQuery } from "@/lib/bindings";
import { MIN_DOC_TEXT_QUERY } from "@/lib/queries";
import { asLibraryBox, boxQuery } from "./libraryBoxes";
import type { LibrarySearch } from "./librarySearch";

export const DOC_PAGE_SIZE = 50;

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
    sort: hasText ? "relevance" : (search.sort ?? "mtime"),
    limit: DOC_PAGE_SIZE,
    // The box wins on its own dimension.
    ...boxQuery(asLibraryBox(search.box)),
  };
}
