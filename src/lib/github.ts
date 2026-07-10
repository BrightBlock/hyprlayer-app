/**
 * Parsing GitHub identifiers out of clone sources — a URL
 * (`https://github.com/acme/repo.git`, `git@github.com:acme/repo.git`,
 * `ssh://git@github.com/acme/repo`) or a bare `owner/repo`.
 *
 * The host is anchored to a string start, `@`, or `/` so that a host merely
 * *containing* `github.com` (`notgithub.com`) does not match.
 */

export type RepoRef = { owner: string; name: string };

/** `owner/name`, end-anchored, with an optional `.git` and trailing slash. */
const URL_REF = /(?:^|[@/])github\.com[/:]([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/i;
/** The owner alone, tolerating any trailing path (`/tree/main`, `/pull/3`). */
const URL_OWNER = /(?:^|[@/])github\.com[/:]([\w.-]+)\/[^/\s]+/i;
const BARE_REF = /^([\w.-]+)\/([\w.-]+)$/;

/** The `{owner, name}` a clone source points at, or null if it isn't GitHub. */
export function githubRepoRef(source: string | null | undefined): RepoRef | null {
  if (!source) return null;
  const s = source.trim();
  const m = s.match(URL_REF) ?? s.match(BARE_REF);
  return m ? { owner: m[1], name: m[2] } : null;
}

/**
 * The GitHub owner (user or org) of a clone source. Returns null for non-GitHub
 * or unparseable sources, so the caller falls back to a letter tile.
 */
export function githubOwner(source: string | null | undefined): string | null {
  if (!source) return null;
  const s = source.trim();
  return s.match(URL_OWNER)?.[1] ?? githubRepoRef(s)?.owner ?? null;
}
