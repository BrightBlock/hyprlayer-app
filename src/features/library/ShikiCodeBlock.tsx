// Chat-pane fenced code block. Calls Shiki on each `code` change and renders
// the resulting HAST as JSX (no dangerouslySetInnerHTML). Falls back to plain
// monospace text while grammar loads or if the language is unknown.
//
// Streaming behavior: during a stream, `code` grows with each token. We
// debounce the highlight call so Shiki only runs once the stream pauses (or
// ends) — the user sees plain monospace while tokens are arriving and the
// block snaps to highlighted shortly after the stream settles. We also gate
// the displayed tree on `highlighted.code === code` so a stale highlight from
// an in-flight call never paints over the current text.

import type { Root } from "hast";
import { toJsxRuntime } from "hast-util-to-jsx-runtime";
import { Fragment, memo, useEffect, useMemo, useState } from "react";
import { jsx, jsxs } from "react/jsx-runtime";

import { ClipboardIcon } from "@/components/icons";
import { useCopyFeedback } from "@/lib/clipboard";
import { highlightToHast, resolveLang } from "./shikiHighlight";

// 150ms is long enough that streaming never triggers a highlight mid-token
// but short enough that a static message snaps to color almost instantly.
const HIGHLIGHT_DEBOUNCE_MS = 150;

function renderHast(root: Root) {
  return toJsxRuntime(root, { Fragment, jsx, jsxs });
}

function ShikiCodeBlockImpl({
  code,
  lang: rawLang,
}: {
  code: string;
  lang: string | undefined;
}) {
  const [highlighted, setHighlighted] = useState<{ hast: Root; code: string } | null>(null);

  useEffect(() => {
    let alive = true;
    const timer = window.setTimeout(() => {
      void highlightToHast(code, rawLang).then((result) => {
        if (!alive || !result) return;
        setHighlighted({ hast: result.hast, code });
      });
    }, HIGHLIGHT_DEBOUNCE_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [code, rawLang]);

  // toJsxRuntime walks the whole hast tree allocating React elements per token,
  // so we memoize by hast identity — unrelated transcript re-renders cost
  // nothing once a block is highlighted.
  const tree = useMemo(
    () => (highlighted && highlighted.code === code ? renderHast(highlighted.hast) : null),
    [highlighted, code],
  );

  const langLabel = resolveLang(rawLang);

  const { copied, trigger: handleCopy } = useCopyFeedback();

  return (
    <div className="group relative my-3 overflow-hidden rounded-md border border-edge bg-panel">
      <div className="flex items-center justify-between border-b border-edge bg-panel-2/40 px-3 py-1">
        <span className="font-mono text-[10px] uppercase tracking-wide text-ink-dim">
          {langLabel}
        </span>
        <button
          type="button"
          onClick={() => handleCopy(code)}
          className="flex cursor-pointer items-center gap-1 rounded px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-ink-dim opacity-0 transition hover:bg-edge hover:text-ink group-hover:opacity-100"
          aria-label="Copy code"
        >
          <ClipboardIcon size={10} />
          <span>{copied ? "copied" : "copy"}</span>
        </button>
      </div>
      <div className="overflow-x-auto px-3 py-2 font-mono text-xs leading-relaxed [&_code]:!bg-transparent [&_code]:!p-0 [&_pre]:!m-0 [&_pre]:!bg-transparent [&_pre]:!p-0">
        {tree ?? (
          <pre>
            <code>{code}</code>
          </pre>
        )}
      </div>
    </div>
  );
}

export const ShikiCodeBlock = memo(ShikiCodeBlockImpl);
