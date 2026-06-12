/**
 * Split a leading `---` frontmatter block off a markdown document.
 * Returns the raw block content (without delimiters) and the remaining body;
 * `frontmatter` is null when the document has no leading block.
 */
export function splitFrontmatter(content: string): { frontmatter: string | null; body: string } {
  const match = /^---\r?\n(?:([\s\S]*?)\r?\n)?---(?:\r?\n|$)/.exec(content);
  if (!match) return { frontmatter: null, body: content };
  return { frontmatter: match[1] ?? "", body: content.slice(match[0].length) };
}

/** Parse a comma-separated tags input into a trimmed, non-empty list. */
export function parseTagsInput(input: string): string[] {
  return input
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}
