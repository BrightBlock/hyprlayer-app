import { useEffect, useRef, useState } from "react";

import { MergeView, unifiedMergeView } from "@codemirror/merge";
import { EditorView } from "@codemirror/view";

import { ErrorNote, Spinner } from "@/components/ui";
import { useFileSides } from "@/lib/queries";
import { diffWordHighlight, readOnlyExtensions, useLangForPath } from "./codeView";

type DiffMode = "unified" | "split";

const MODE_KEY = "hyprlayer:diff-mode";

function loadMode(): DiffMode {
  return localStorage.getItem(MODE_KEY) === "split" ? "split" : "unified";
}

/** Rich read-only diff of one worktree file, shown as a main-pane tab. Uses
 *  CodeMirror's merge view (the frontend computes the diff from the two file
 *  texts the backend returns). Unified by default; toggles to side-by-side. */
export function DiffTab({ id, path, staged }: { id: string; path: string; staged: boolean }) {
  const sides = useFileSides(id, path, staged);
  const [mode, setMode] = useState<DiffMode>(loadMode);
  const lang = useLangForPath(path);
  const hostRef = useRef<HTMLDivElement>(null);

  const data = sides.data;
  // A file that exists on only one side (added/deleted) has nothing to lay out
  // side-by-side, so we pin those to unified and disable the toggle.
  const oneSided = !!data && (!data.originalExists || !data.modifiedExists);
  const effectiveMode: DiffMode = oneSided ? "unified" : mode;

  // (Re)mount the CodeMirror view whenever the content, mode, or language changes.
  useEffect(() => {
    const host = hostRef.current;
    if (!host || !data) return;

    // `diffWordHighlight` replaces @codemirror/merge's `highlightChanges` (kept
    // off) so only genuinely changed words inside a modified line are tinted,
    // wholly-new/removed lines get the line background alone, GitHub-style.
    const readOnly = [readOnlyExtensions(lang), diffWordHighlight];
    const collapseUnchanged = { margin: 3, minSize: 6 };

    let view: EditorView | MergeView;
    if (effectiveMode === "split") {
      view = new MergeView({
        parent: host,
        a: { doc: data.original, extensions: readOnly },
        b: { doc: data.modified, extensions: readOnly },
        highlightChanges: false,
        gutter: true,
        collapseUnchanged,
      });
    } else {
      view = new EditorView({
        parent: host,
        doc: data.modified,
        extensions: [
          readOnly,
          unifiedMergeView({
            original: data.original,
            highlightChanges: false,
            gutter: true,
            mergeControls: false,
            collapseUnchanged,
          }),
        ],
      });
    }
    return () => view.destroy();
  }, [data, effectiveMode, lang]);

  const setModePersist = (next: DiffMode) => {
    localStorage.setItem(MODE_KEY, next);
    setMode(next);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-edge px-4">
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink-dim" title={path}>
          {path}
        </span>
        <span className="shrink-0 rounded bg-edge/60 px-1.5 py-0.5 text-[10px] font-medium text-ink-dim">
          {staged ? "staged" : "working"}
        </span>
        <div className="flex shrink-0 overflow-hidden rounded-md border border-edge">
          {(["unified", "split"] as const).map((m) => (
            <button
              key={m}
              type="button"
              disabled={oneSided}
              onClick={() => setModePersist(m)}
              className={`px-2 py-0.5 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
                effectiveMode === m ? "bg-panel-2 text-ink" : "text-ink-dim hover:bg-panel-2"
              }`}
            >
              {m === "unified" ? "Unified" : "Side by side"}
            </button>
          ))}
        </div>
      </div>
      {sides.isLoading ? (
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      ) : sides.isError ? (
        <div className="p-4">
          <ErrorNote error={sides.error} />
        </div>
      ) : (
        <div ref={hostRef} className="selectable min-h-0 flex-1 overflow-auto" />
      )}
    </div>
  );
}
