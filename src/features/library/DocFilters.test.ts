import { describe, expect, it } from "vitest";

import { DOC_PAGE_SIZE, docQueryFromSearch, splitParam } from "./DocFilters";
import type { LibrarySearch } from "./librarySearch";

function search(q: string | undefined): LibrarySearch {
  return { q };
}

describe("docQueryFromSearch", () => {
  it("sends no text query below MIN_DOC_TEXT_QUERY (length 0, 1, 2)", () => {
    for (const q of [undefined, "", "a", "au"]) {
      const query = docQueryFromSearch(search(q));
      expect(query.text).toBeNull();
      expect(query.sort).toBe("mtime");
    }
  });

  it("sends the text query at MIN_DOC_TEXT_QUERY (length 3)", () => {
    const query = docQueryFromSearch(search("aut"));
    expect(query.text).toBe("aut");
    expect(query.sort).toBe("relevance");
  });

  it("keeps sending the text query above the threshold", () => {
    const query = docQueryFromSearch(search("authentication"));
    expect(query.text).toBe("authentication");
    expect(query.sort).toBe("relevance");
  });

  it("passes non-text filters through unchanged regardless of query length", () => {
    for (const q of [undefined, "a", "authentication"]) {
      const query = docQueryFromSearch({
        q,
        type: "plan,research",
        status: "active",
        project: "myproj",
        scope: "shared",
        author: "jt",
        tag: "security,backend",
      });
      expect(query.types).toEqual(["plan", "research"]);
      expect(query.statuses).toEqual(["active"]);
      expect(query.projects).toEqual(["myproj"]);
      expect(query.scopes).toEqual(["shared"]);
      expect(query.authors).toEqual(["jt"]);
      expect(query.tags).toEqual(["security", "backend"]);
      expect(query.tagMode).toBe("or");
      expect(query.limit).toBe(DOC_PAGE_SIZE);
    }
  });
});

describe("splitParam", () => {
  it("splits, trims, and drops empties", () => {
    expect(splitParam("a, b ,,c")).toEqual(["a", "b", "c"]);
  });

  it("returns an empty array for undefined", () => {
    expect(splitParam(undefined)).toEqual([]);
  });
});
