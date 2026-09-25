
import type { Root } from "hast";
import { toJsxRuntime } from "hast-util-to-jsx-runtime";
import { Fragment, memo, useEffect, useMemo, useState } from "react";
import { jsx, jsxs } from "react/jsx-runtime";

import { Icon } from "@/components/Icon";
import { useCopyFeedback } from "@/lib/clipboard";
import { highlightToHast, resolveLang } from "./shikiHighlight";

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

  const tree = useMemo(
    () => (highlighted && highlighted.code === code ? renderHast(highlighted.hast) : null),
    [highlighted, code],
  );

  const langLabel = resolveLang(rawLang);

  const { copied, trigger: handleCopy } = useCopyFeedback();

  return (
    <div className="group relative my-3 overflow-hidden rounded-md border border-edge bg-panel">
      <div className="flex items-center justify-between border-b border-edge bg-panel-2/40 px-3 py-1">
        <span className="text-label text-ink-dim">
          {langLabel}
        </span>
        <button
          type="button"
          onClick={() => handleCopy(code)}
          className="flex cursor-pointer items-center gap-1 rounded px-1.5 py-0.5 text-label text-ink-dim opacity-0 transition hover:bg-edge hover:text-ink group-hover:opacity-100"
          aria-label="Copy code"
        >
          <Icon name="copy" size="xs" />
          <span>{copied ? "copied" : "copy"}</span>
        </button>
      </div>
      <div className="overflow-x-auto px-3 py-2 text-meta [&_code]:!bg-transparent [&_code]:!p-0 [&_pre]:!m-0 [&_pre]:!bg-transparent [&_pre]:!p-0">
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
