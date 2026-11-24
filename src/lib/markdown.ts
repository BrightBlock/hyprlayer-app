import MarkdownIt, { type Token } from "markdown-it";

// markdown-it over react-markdown: ~4MB vs ~38MB of garbage per 150KB doc (see markdown.test.ts budget).
export type CodeBlock = { code: string; lang: string | undefined };
type MarkdownHeading = { level: number; text: string; line: number };
type RenderedMarkdown = { html: string; codeBlocks: CodeBlock[]; headings: MarkdownHeading[] };

const LINE_STAMPED = new Set([
  "heading_open",
  "paragraph_open",
  "bullet_list_open",
  "ordered_list_open",
  "list_item_open",
  "blockquote_open",
  "hr",
  "table_open",
  "fence",
  "code_block",
]);
const TASK = /^\[([ xX])\][ \t]+/;

// html: false escapes raw HTML, which matters because the preview assigns innerHTML.
const md = new MarkdownIt({ html: false, linkify: true });

md.core.ruler.after("block", "data_line", (state) => {
  for (const token of state.tokens) {
    if (token.map && LINE_STAMPED.has(token.type)) token.attrSet("data-line", String(token.map[0] + 1));
  }
});

md.core.ruler.after("inline", "task_items", (state) => {
  const tokens = state.tokens;
  for (let i = 2; i < tokens.length; i++) {
    const first = tokens[i].children?.[0];
    if (tokens[i - 2].type !== "list_item_open" || first?.type !== "text") continue;
    const match = TASK.exec(first.content);
    if (!match) continue;
    first.content = first.content.slice(match[0].length);
    const box = new state.Token("html_inline", "", 0);
    box.content = `<input type="checkbox" disabled${match[1] === " " ? "" : " checked"}> `;
    tokens[i].children!.unshift(box);
  }
});

const rules = md.renderer.rules;

rules.link_open = (tokens, idx, options, _env, self) => {
  tokens[idx].attrSet("target", "_blank");
  tokens[idx].attrSet("rel", "noreferrer");
  return self.renderToken(tokens, idx, options);
};

rules.table_open = (tokens, idx) => `<div class="md-table" data-line="${tokens[idx].attrGet("data-line")}"><table>\n`;
rules.table_close = () => "</table></div>\n";

// Fences render as empty placeholders; MarkdownPreview mounts ShikiCodeBlock into each.
rules.fence = rules.code_block = (tokens, idx, _options, env) => {
  const token = tokens[idx];
  const codeBlocks = (env as { codeBlocks: CodeBlock[] }).codeBlocks;
  const lang = token.info.trim().split(/\s+/)[0] || undefined;
  codeBlocks.push({ code: token.content.replace(/\n+$/, ""), lang });
  return `<div data-line="${token.attrGet("data-line")}" data-code-block="${codeBlocks.length - 1}"></div>\n`;
};

function headingsOf(tokens: Token[]): MarkdownHeading[] {
  const out: MarkdownHeading[] = [];
  for (let i = 0; i < tokens.length; i++) {
    const open = tokens[i];
    if (open.type !== "heading_open" || !open.map) continue;
    let text = "";
    for (const child of tokens[i + 1].children ?? []) {
      if (child.type === "text" || child.type === "code_inline") text += child.content;
      else if (child.type === "softbreak") text += " ";
    }
    out.push({ level: Number(open.tag.slice(1)), text: text.trim(), line: open.map[0] + 1 });
  }
  return out;
}

// One entry, and only the output: the doc view reads the same body for its headings and
// its preview, while tokens (~5MB per 150KB doc) or older docs would pin memory.
let cached: { body: string; rendered: RenderedMarkdown } | null = null;

export function renderMarkdown(body: string): RenderedMarkdown {
  if (cached?.body === body) return cached.rendered;
  const env = { codeBlocks: [] as CodeBlock[] };
  const tokens = md.parse(body, env);
  const rendered = { html: md.renderer.render(tokens, md.options, env), codeBlocks: env.codeBlocks, headings: headingsOf(tokens) };
  cached = { body, rendered };
  return rendered;
}
