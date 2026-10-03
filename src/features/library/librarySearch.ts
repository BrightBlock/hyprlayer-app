import { MIN_DOC_TEXT_QUERY } from "@/lib/queries";

export type LibrarySearch = {
  type?: string;
  status?: string;
  scope?: string;
  project?: string;
  author?: string;
  tag?: string;
  q?: string;
  doc?: string;
  from?: string;
};

// `project` scopes the library; it is not a filter.
export const libraryFiltersActive = (s: LibrarySearch): boolean =>
  Boolean(s.type || s.status || s.scope || s.author || s.tag) || (s.q?.length ?? 0) >= MIN_DOC_TEXT_QUERY;
