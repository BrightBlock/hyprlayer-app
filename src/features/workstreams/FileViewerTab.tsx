import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import CodeMirror from "@uiw/react-codemirror";
import { Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";

import { CheckIcon, PencilIcon, SearchIcon } from "@/components/icons";
import { ErrorNote, Spinner } from "@/components/ui";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";
import { splitFrontmatter } from "@/features/library/text";
import { clearActiveFileSave, setActiveFileSave } from "@/lib/fileSave";
import { IS_MACOS } from "@/lib/platform";
import { useSaveWorktreeFile, useWorktreeFile } from "@/lib/queries";
import { codeBase, useLangForPath } from "./codeView";

const MARKDOWN_RE = /\.(md|markdown|mdx)$/i;
// A NUL byte marks a binary file the read lossily decoded; keep those read-only.
const NUL = String.fromCharCode(0);

// codeBase supplies the gutter + syntax highlighting, so drop the defaults.
const BASIC_SETUP = {
  foldGutter: true,
  lineNumbers: false,
  syntaxHighlighting: false,
  highlightActiveLine: true,
} as const;

/** Editable CodeMirror view of a worktree file, opened from the "All Files"
 *  tree. Cmd/Ctrl+S saves; markdown files get a raw⇄preview toggle. */
export function FileViewerTab({ id, path }: { id: string; path: string }) {
  const file = useWorktreeFile(id, path);

  if (file.isLoading) {
    return (
      <Shell path={path}>
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      </Shell>
    );
  }
  if (file.isError) {
    return (
      <Shell path={path}>
        <div className="p-4">
          <ErrorNote error={file.error} />
        </div>
      </Shell>
    );
  }
  if (!file.data) return null;

  // Keyed on file identity, not mtime, so a save doesn't remount and drop the cursor.
  return (
    <FileEditor
      key={`${id}:${path}`}
      id={id}
      path={path}
      initialContent={file.data.content}
      initialMtimeMs={file.data.mtimeMs ?? 0}
    />
  );
}

/** The pane frame: a path bar (with optional controls) over the body. */
function Shell({ path, controls, children }: { path: string; controls?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-edge px-4">
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink-dim" title={path}>
          {path}
        </span>
        {controls}
      </div>
      {children}
    </div>
  );
}

type EditorProps = {
  id: string;
  path: string;
  initialContent: string;
  initialMtimeMs: number;
};

function FileEditor({ id, path, initialContent, initialMtimeMs }: EditorProps) {
  const lang = useLangForPath(path);
  const save = useSaveWorktreeFile(id, path);

  const [content, setContent] = useState(initialContent);
  // The last-saved baseline; `dirty` is the buffer diverging from it.
  const [savedContent, setSavedContent] = useState(initialContent);
  const [baseMtimeMs, setBaseMtimeMs] = useState(initialMtimeMs);
  const [preview, setPreview] = useState(false);
  const [justSaved, setJustSaved] = useState(false);

  const dirty = content !== savedContent;
  const isMarkdown = MARKDOWN_RE.test(path);
  const editable = useMemo(() => !initialContent.includes(NUL), [initialContent]);

  // Transient "Saved" flash.
  const savedTimer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (savedTimer.current) window.clearTimeout(savedTimer.current);
    },
    [],
  );

  // Read via ref by the keymap and the File-menu registry to avoid a stale closure.
  const saveRef = useRef<() => void>(() => {});

  function doSave() {
    if (!editable || !dirty || save.isPending) return;
    const snapshot = content;
    save.mutate(
      { content: snapshot, baseMtimeMs },
      {
        onSuccess: (newMtimeMs) => {
          setSavedContent(snapshot);
          setBaseMtimeMs(newMtimeMs ?? 0);
          setJustSaved(true);
          if (savedTimer.current) window.clearTimeout(savedTimer.current);
          savedTimer.current = window.setTimeout(() => setJustSaved(false), 1800);
        },
      },
    );
  }
  saveRef.current = doSave;

  // Active target for File ▸ Save; re-assert on focus, release on unmount.
  const tokenRef = useRef<object>({});
  const registerActive = () => setActiveFileSave(tokenRef.current, () => saveRef.current());
  useEffect(() => {
    const token = tokenRef.current;
    setActiveFileSave(token, () => saveRef.current());
    return () => clearActiveFileSave(token);
  }, []);

  const extensions = useMemo(() => {
    const exts = [...codeBase, EditorView.lineWrapping, ...(lang ? [lang] : [])];
    // On macOS the native File ▸ Save accelerator handles ⌘S; only bind it here elsewhere.
    if (!IS_MACOS) {
      exts.push(
        Prec.high(
          keymap.of([
            {
              key: "Mod-s",
              preventDefault: true,
              run: () => {
                saveRef.current();
                return true;
              },
            },
          ]),
        ),
      );
    }
    return exts;
  }, [lang]);

  const controls = (
    <>
      <SaveStatus saving={save.isPending} dirty={dirty} justSaved={justSaved} readOnly={!editable} />
      {isMarkdown && (
        <button
          type="button"
          onClick={() => setPreview((p) => !p)}
          aria-pressed={preview}
          className={`inline-flex h-7 w-7 items-center justify-center rounded-md ${
            preview ? "bg-accent/20 text-accent" : "text-ink-dim hover:bg-edge/60 hover:text-ink"
          }`}
          title={preview ? "Edit raw" : "Preview"}
          aria-label={preview ? "Edit raw" : "Preview"}
        >
          {preview ? <PencilIcon size={15} /> : <SearchIcon size={15} />}
        </button>
      )}
    </>
  );

  return (
    <Shell path={path} controls={controls}>
      {save.isError && (
        <div className="shrink-0 border-b border-edge p-3">
          <ErrorNote error={save.error} />
        </div>
      )}
      {preview ? (
        <div className="selectable min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto max-w-3xl px-8 py-6">
            <MarkdownPreview body={splitFrontmatter(content).body} />
          </div>
        </div>
      ) : (
        <div className="selectable min-h-0 flex-1 overflow-hidden" onFocus={registerActive}>
          <CodeMirror
            value={content}
            className="h-full"
            height="100%"
            theme="none"
            editable={editable}
            extensions={extensions}
            basicSetup={BASIC_SETUP}
            onChange={(value) => setContent(value)}
          />
        </div>
      )}
    </Shell>
  );
}

/** Save-state indicator: unsaved dot, saving spinner, transient "Saved" check. */
function SaveStatus({
  saving,
  dirty,
  justSaved,
  readOnly,
}: {
  saving: boolean;
  dirty: boolean;
  justSaved: boolean;
  readOnly: boolean;
}) {
  if (readOnly) return <span className="text-[11px] text-ink-dim">read-only</span>;
  if (saving) {
    return (
      <span className="flex items-center gap-1 text-[11px] text-ink-dim">
        <Spinner size={12} />
        Saving…
      </span>
    );
  }
  if (dirty) {
    return (
      <span className="flex items-center gap-1 text-[11px] text-warn">
        <span className="h-1.5 w-1.5 rounded-full bg-warn" aria-hidden="true" />
        Unsaved
      </span>
    );
  }
  if (justSaved) {
    return (
      <span className="flex items-center gap-1 text-[11px] text-ok">
        <CheckIcon size={12} />
        Saved
      </span>
    );
  }
  return null;
}
