
export type RepoRef = { owner: string; name: string };

const URL_REF = /(?:^|[@/])github\.com[/:]([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/i;
const URL_OWNER = /(?:^|[@/])github\.com[/:]([\w.-]+)\/[^/\s]+/i;
const BARE_REF = /^([\w.-]+)\/([\w.-]+)$/;

export function githubRepoRef(source: string | null | undefined): RepoRef | null {
  if (!source) return null;
  const s = source.trim();
  const m = s.match(URL_REF) ?? s.match(BARE_REF);
  return m ? { owner: m[1], name: m[2] } : null;
}

export function githubOwner(source: string | null | undefined): string | null {
  if (!source) return null;
  const s = source.trim();
  return s.match(URL_OWNER)?.[1] ?? githubRepoRef(s)?.owner ?? null;
}
