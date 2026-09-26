import { memo } from "react";
import type { Element } from "hast";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { ShikiCodeBlock } from "./ShikiCodeBlock";


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

// DocDetail maps a click in the rendered doc back to its source line.
const line = (node: Element | undefined) => node?.position?.start.line;

const components: Components = {
  h1: ({ children, node }) => (
    <h1 data-line={line(node)} className="mb-3 mt-6 border-b border-edge pb-1 text-h2 first:mt-0">
      {children}
    </h1>
  ),
  h2: ({ children, node }) => (
    <h2 data-line={line(node)} className="mb-2 mt-6 text-h3 first:mt-0">{children}</h2>
  ),
  h3: ({ children, node }) => (
    <h3 data-line={line(node)} className="mb-2 mt-4 text-title first:mt-0">{children}</h3>
  ),
  h4: ({ children, node }) => (
    <h4 data-line={line(node)} className="mb-1 mt-4 text-body font-semibold first:mt-0">{children}</h4>
  ),
  p: ({ children, node }) => (
    <p data-line={line(node)} className="my-3">
      {children}
    </p>
  ),
  ul: ({ children, node }) => (
    <ul data-line={line(node)} className="my-3 list-disc space-y-1 pl-6">
      {children}
    </ul>
  ),
  ol: ({ children, node }) => (
    <ol data-line={line(node)} className="my-3 list-decimal space-y-1 pl-6">
      {children}
    </ol>
  ),
  li: ({ children, node }) => <li data-line={line(node)}>{children}</li>,
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
  blockquote: ({ children, node }) => (
    <blockquote data-line={line(node)} className="my-3 border-l-2 border-edge pl-3 text-ink-dim">{children}</blockquote>
  ),
  code: ({ children }) => (
    <code className="rounded bg-panel-2 px-1 py-0.5 font-mono text-[0.85em]">{children}</code>
  ),
  pre: ({ children, node }) => {
    const inner = node?.children?.find(
      (c): c is Element => c.type === "element" && c.tagName === "code",
    );
    if (inner) {
      const { code, lang } = readFencedCode(inner);
      return (
        <div data-line={line(node)}>
          <ShikiCodeBlock code={code} lang={lang} />
        </div>
      );
    }
    return (
      <pre data-line={line(node)} className="my-3 overflow-x-auto rounded-md border border-edge bg-panel p-3 text-meta">
        {children}
      </pre>
    );
  },
  hr: ({ node }) => <hr data-line={line(node)} className="my-6 border-edge" />,
  table: ({ children, node }) => (
    <div data-line={line(node)} className="my-3 overflow-x-auto">
      <table className="w-full border-collapse text-body">{children}</table>
    </div>
  ),
  th: ({ children }) => (
    <th className="border border-edge bg-panel-2 px-2 py-1 text-left font-medium">{children}</th>
  ),
  td: ({ children }) => <td className="border border-edge px-2 py-1 align-top">{children}</td>,
};

const remarkPlugins = [remarkGfm];

export const MarkdownPreview = memo(function MarkdownPreview({ body }: { body: string }) {
  return (
    <div className="selectable text-body-lg text-ink">
      <ReactMarkdown remarkPlugins={remarkPlugins} components={components}>
        {body}
      </ReactMarkdown>
    </div>
  );
});
