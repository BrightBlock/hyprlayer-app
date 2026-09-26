import { memo, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { type CodeBlock, renderMarkdown } from "@/lib/markdown";
import { ShikiCodeBlock } from "./ShikiCodeBlock";

type Slot = { el: Element; block: CodeBlock };

// innerHTML instead of a React tree: a 150KB doc is ~9k nodes, and React would hold a fiber for each.
export const MarkdownPreview = memo(function MarkdownPreview({ body }: { body: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [slots, setSlots] = useState<Slot[]>([]);
  // Per instance: other mounted previews evict the shared cache, and a fresh result would loop the effect.
  const { html, codeBlocks } = useMemo(() => renderMarkdown(body), [body]);

  useLayoutEffect(() => {
    const root = ref.current!;
    root.innerHTML = html;
    setSlots(Array.from(root.querySelectorAll("[data-code-block]"), (el, i) => ({ el, block: codeBlocks[i] })));
  }, [html, codeBlocks]);

  return (
    <>
      <div ref={ref} className="md selectable text-body-lg text-ink" />
      {slots.map(({ el, block }, i) => createPortal(<ShikiCodeBlock code={block.code} lang={block.lang} />, el, String(i)))}
    </>
  );
});
