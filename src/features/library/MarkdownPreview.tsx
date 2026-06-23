import { memo } from "react";
import type { Element } from "hast";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { ShikiCodeBlock } from "./ShikiCodeBlock";

// Hand-rolled prose styling (no typography plugin). The `pre` slot intercepts
// fenced code blocks and routes them through ShikiCodeBlock; everything else
// is plain JSX with Tailwind classes bound to the theme tokens.

// Pull `{ code, lang }` off the inner <code> of a fenced block. CommonMark/GFM
// guarantees a single text child, so no recursion. Trailing newlines (the ones
// the closing ``` is on) are stripped so the block doesn't render an empty
// final line.
function readFencedCode(inner: Element): { code: string; lang: string | undefined } {
  const first = inner.children[0];
  const code = first?.type === "text" ? first.value.replace(/\n+$/, "") : "";
  const cls = inner.properties?.className;
  const list = Array.isArray(cls) ? cls : [];
  const langClass = list.find(
    (c): c is string => typeof c === "string" && c.startsWith("language-"),
  );
  return { code, lang: langClass?.slice("language-".length) };
}

const components: Components = {
  h1: ({ children }) => (
    <h1 className="mb-3 mt-6 border-b border-edge pb-1 text-2xl font-semibold first:mt-0">
      {children}
    </h1>
  ),
  h2: ({ children }) => (
    <h2 className="mb-2 mt-6 text-xl font-semibold first:mt-0">{children}</h2>
  ),
  h3: ({ children }) => (
    <h3 className="mb-2 mt-4 text-base font-semibold first:mt-0">{children}</h3>
  ),
  h4: ({ children }) => (
    <h4 className="mb-1 mt-4 text-sm font-semibold first:mt-0">{children}</h4>
  ),
  p: ({ children }) => <p className="my-3 leading-relaxed">{children}</p>,
  ul: ({ children }) => <ul className="my-3 list-disc space-y-1 pl-6">{children}</ul>,
  ol: ({ children }) => <ol className="my-3 list-decimal space-y-1 pl-6">{children}</ol>,
  li: ({ children }) => <li className="leading-relaxed">{children}</li>,
  a: ({ children, href }) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="text-ink underline decoration-ink-dim/50 hover:decoration-ink"
    >
      {children}
    </a>
  ),
  blockquote: ({ children }) => (
    <blockquote className="my-3 border-l-2 border-edge pl-3 text-ink-dim">{children}</blockquote>
  ),
  // Inline code only — fenced blocks are caught in the `pre` slot below.
  code: ({ children }) => (
    <code className="rounded bg-panel-2 px-1 py-0.5 font-mono text-[0.85em]">{children}</code>
  ),
  // Fenced code block. The HAST emitted by react-markdown for ``` blocks is
  // <pre><code class="language-xyz">text</code></pre> — we pull lang + raw text
  // off the inner <code> and hand it to ShikiCodeBlock. Any non-code <pre>
  // (rare in markdown output) falls back to a styled container.
  pre: ({ children, node }) => {
    const inner = node?.children?.find(
      (c): c is Element => c.type === "element" && c.tagName === "code",
    );
    if (inner) {
      const { code, lang } = readFencedCode(inner);
      return <ShikiCodeBlock code={code} lang={lang} />;
    }
    return (
      <pre className="my-3 overflow-x-auto rounded-md border border-edge bg-panel p-3 font-mono text-xs leading-relaxed">
        {children}
      </pre>
    );
  },
  hr: () => <hr className="my-6 border-edge" />,
  table: ({ children }) => (
    <div className="my-3 overflow-x-auto">
      <table className="w-full border-collapse text-sm">{children}</table>
    </div>
  ),
  th: ({ children }) => (
    <th className="border border-edge bg-panel-2 px-2 py-1 text-left font-medium">{children}</th>
  ),
  td: ({ children }) => <td className="border border-edge px-2 py-1 align-top">{children}</td>,
};

// Hoisted so ReactMarkdown sees a stable plugins array across renders.
const remarkPlugins = [remarkGfm];

// Memoized: react-markdown re-parses the whole body (mdast → hast) on every
// render, so a transcript full of settled messages must not re-parse them when
// an unrelated streamed token arrives. With stable `body` this is a no-op.
export const MarkdownPreview = memo(function MarkdownPreview({ body }: { body: string }) {
  return (
    <div className="selectable text-sm text-ink">
      <ReactMarkdown remarkPlugins={remarkPlugins} components={components}>
        {body}
      </ReactMarkdown>
    </div>
  );
});
