import { useEffect, useRef } from "react";

import { EditorView } from "@codemirror/view";

import { ErrorNote, Spinner } from "@/components/ui";
import { useWorktreeFile } from "@/lib/queries";
import { readOnlyExtensions, useLangForPath } from "./codeView";

/** Read-only CodeMirror view of an arbitrary worktree file, opened from the
 *  "All Files" tree as a main-pane tab. Shares the theme + language detection
 *  with the diff viewer. */
export function FileViewerTab({ id, path }: { id: string; path: string }) {
  const file = useWorktreeFile(id, path);
  const lang = useLangForPath(path);
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host || file.data === undefined) return;
    const view = new EditorView({
      parent: host,
      doc: file.data,
      extensions: readOnlyExtensions(lang),
    });
    return () => view.destroy();
  }, [file.data, lang]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-9 shrink-0 items-center border-b border-edge px-4">
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink-dim" title={path}>
          {path}
        </span>
      </div>
      {file.isLoading ? (
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      ) : file.isError ? (
        <div className="p-4">
          <ErrorNote error={file.error} />
        </div>
      ) : (
        <div ref={hostRef} className="selectable min-h-0 flex-1 overflow-auto" />
      )}
    </div>
  );
}
