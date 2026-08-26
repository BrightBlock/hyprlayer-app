export function splitFrontmatter(content: string): { frontmatter: string | null; body: string } {
  const match = /^---\r?\n(?:([\s\S]*?)\r?\n)?---(?:\r?\n|$)/.exec(content);
  if (!match) return { frontmatter: null, body: content };
  return { frontmatter: match[1] ?? "", body: content.slice(match[0].length) };
}

export function docTitle(meta: { relPath: string; fm: { title?: string | null } }): string {
  return meta.fm.title ?? meta.relPath.split("/").pop() ?? meta.relPath;
}

export function parseTagsInput(input: string): string[] {
  return input
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}
