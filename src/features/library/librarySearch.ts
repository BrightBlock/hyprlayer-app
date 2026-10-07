import { MIN_DOC_TEXT_QUERY } from "@/lib/queries";
import type { LibraryBox } from "./libraryBoxes";

export const LIBRARY_SORTS = ["mtime", "date", "title"] as const;
export type LibrarySort = (typeof LIBRARY_SORTS)[number];

export type LibrarySearch = {
  box?: LibraryBox;
  // Unset is mtime; text of MIN_DOC_TEXT_QUERY+ chars sorts by relevance regardless.
  sort?: LibrarySort;
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

export const listFiltersActive = (s: LibrarySearch): boolean =>
  Boolean(s.type || s.status || s.scope || s.author || s.tag);

// `project` scopes the library; it is not a filter. A box is, so an empty New box isn't an empty library.
export const libraryFiltersActive = (s: LibrarySearch): boolean =>
  Boolean(s.type || s.status || s.scope || s.author || s.tag || (s.box && s.box !== "all")) ||
  (s.q?.length ?? 0) >= MIN_DOC_TEXT_QUERY;
