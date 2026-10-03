import { useEffect, useRef, useState } from "react";

import { MergeView, unifiedMergeView } from "@codemirror/merge";
import type { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

import { diffWordHighlight, readOnlyExtensions } from "@/lib/codeView";

export type DiffMode = "unified" | "split";

const MODE_KEY = "hyprlayer:diff-mode";

function loadMode(): DiffMode {
  return localStorage.getItem(MODE_KEY) === "split" ? "split" : "unified";
}

export function useDiffMode(): [DiffMode, (m: DiffMode) => void] {
  const [mode, setMode] = useState<DiffMode>(loadMode);
  const setModePersist = (next: DiffMode) => {
    localStorage.setItem(MODE_KEY, next);
    setMode(next);
  };
  return [mode, setModePersist];
}

export type DiffViews = { original: EditorView | null; modified: EditorView };

// One EditorView, or one MergeView in split mode, destroyed whenever the inputs change.
export function DiffView({
  original,
  modified,
  mode,
  lang,
  extensions,
  onViews,
}: {
  original: string;
  modified: string;
  mode: DiffMode;
  lang: Extension | null;
  extensions?: { original?: Extension; modified?: Extension };
  onViews?: (v: DiffViews) => void;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  // Read at construction only: later data goes in through StateEffects, never a rebuild.
  const extensionsRef = useRef(extensions);
  extensionsRef.current = extensions;
  const onViewsRef = useRef(onViews);
  onViewsRef.current = onViews;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const extra = extensionsRef.current;
    const readOnly = [readOnlyExtensions(lang), diffWordHighlight];
    const collapseUnchanged = { margin: 3, minSize: 6 };

    let destroy: () => void;
    if (mode === "split") {
      const view = new MergeView({
        parent: host,
        a: { doc: original, extensions: [readOnly, extra?.original ?? []] },
        b: { doc: modified, extensions: [readOnly, extra?.modified ?? []] },
        highlightChanges: false,
        gutter: true,
        collapseUnchanged,
      });
      onViewsRef.current?.({ original: view.a, modified: view.b });
      destroy = () => view.destroy();
    } else {
      const view = new EditorView({
        parent: host,
        doc: modified,
        extensions: [
          readOnly,
          unifiedMergeView({
            original,
            highlightChanges: false,
            gutter: true,
            mergeControls: false,
            collapseUnchanged,
          }),
          extra?.modified ?? [],
        ],
      });
      onViewsRef.current?.({ original: null, modified: view });
      destroy = () => view.destroy();
    }
    return destroy;
  }, [original, modified, mode, lang]);

  return <div ref={hostRef} className="selectable min-h-0 flex-1 overflow-auto" />;
}
