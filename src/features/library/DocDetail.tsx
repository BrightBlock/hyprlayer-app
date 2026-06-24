import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView } from "@codemirror/view";
import { useMemo, useState } from "react";

import { Badge, ErrorNote, PageError, PageSpinner, Spinner } from "../../components/ui";
import { FolderIcon, PencilIcon, SearchIcon } from "../../components/icons";
import type { DocContent } from "../../lib/bindings";
import {
  useDoc,
  usePatchDocFrontmatter,
  useRevealDoc,
  useSaveDoc,
} from "../../lib/queries";
import { IS_MACOS } from "../../lib/platform";
import { useUiStore } from "../../lib/uiStore";
import { codeBase } from "../workstreams/codeView";
import { FrontmatterPanel } from "./FrontmatterPanel";
import { MarkdownPreview } from "./MarkdownPreview";
import { docTitle, splitFrontmatter } from "./text";

// Editor chrome + syntax colors bound to the app's theme tokens (shared with the
// diff/file viewers via `codeBase`), so the editor recolors with the active
// theme instead of being stuck on a fixed dark palette. `lineWrapping` wraps
// long lines at the editor's width (it tracks the column/window) instead of
// scrolling horizontally; the markdown grammar rides on top for highlighting.
const editorExtensions = [...codeBase, EditorView.lineWrapping, markdown()];

type Props = {
  /** Absolute-path doc id. */
  id: string;
  mode: "edit" | "preview";
  onSetMode: (mode: "edit" | "preview") => void;
};

/**
 * The library's detail pane. Loads the doc, then hands it to {@link DocBody}
 * keyed on the doc's identity *and* mtime: switching documents — or a refetch
 * after a save/frontmatter patch bumps the mtime — mints a fresh editor buffer
 * rather than carrying a stale (possibly dirty) one across, which could
 * otherwise save one document's body over another's.
 */
export function DocDetail({ id, mode, onSetMode }: Props) {
  const doc = useDoc(id);
  if (doc.isLoading) return <PageSpinner />;
  if (doc.isError) return <PageError error={doc.error} />;
  if (!doc.data) return null;
  return (
    <DocBody
      key={`${id}:${doc.data.mtimeMs ?? 0}`}
      id={id}
      doc={doc.data}
      mode={mode}
      onSetMode={onSetMode}
    />
  );
}

type BodyProps = Props & { doc: DocContent };

/**
 * The rendered detail: a path bar (with the edit/preview/save controls), the
 * title, the inline-editable frontmatter card, and the body — rendered prose in
 * preview, raw CodeMirror in edit. Folds in the old full-page DocPage's
 * save/patch/reveal machinery; the body's mtime-guarded save still gates
 * frontmatter patches (the card disables while the buffer is dirty). The editor
 * buffer is seeded from `doc` at mount and lives only as long as this instance —
 * see {@link DocDetail} for why that lifetime is tied to the doc's mtime.
 */
function DocBody({ id, doc, mode, onSetMode }: BodyProps) {
  const saveDoc = useSaveDoc(id);
  const patchFrontmatter = usePatchDocFrontmatter(id);
  const revealDoc = useRevealDoc();
  const docWide = useUiStore((s) => s.docWide);
  const setDocWide = useUiStore((s) => s.setDocWide);
  const { meta, content: seed } = doc;
  const mtimeMs = doc.mtimeMs ?? 0;
  const [content, setContent] = useState(seed);
  const [dirty, setDirty] = useState(false);

  const body = useMemo(() => splitFrontmatter(content).body, [content]);

  // One button does edit ⇄ save: in preview it enters edit mode; in edit it
  // saves (when there are changes) and returns to preview, or just exits when
  // clean. A failed save keeps you in edit so the buffer + error survive.
  function primaryAction() {
    if (mode === "preview") {
      onSetMode("edit");
      return;
    }
    if (!dirty) {
      onSetMode("preview");
      return;
    }
    saveDoc.mutate(
      { content, baseMtimeMs: mtimeMs },
      {
        onSuccess: () => {
          setDirty(false);
          onSetMode("preview");
        },
      },
    );
  }
  const primaryTitle = mode === "preview" ? "Edit" : dirty ? "Save and preview" : "Preview";
  const revealLabel = IS_MACOS ? "Reveal in Finder" : "Show in folder";

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-11 shrink-0 items-center justify-between gap-3 border-b border-edge bg-panel px-6">
        <div className="flex min-w-0 items-center gap-2">
          <span className="selectable truncate font-mono text-xs text-ink-dim">{meta.relPath}</span>
          {mode === "edit" && (
            <span className="shrink-0 text-[11px] font-medium uppercase tracking-wider text-accent">
              • Editing
            </span>
          )}
          {dirty && <Badge tone="warn">unsaved</Badge>}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {/* Narrow ↔ wide layout: a centered readable column or the full pane
              for more real estate (tables, code, side-by-side). Both states are
              shown so the choice is obvious; persisted, so it sticks across docs
              and applies to preview + the editor alike. */}
          <div
            role="group"
            aria-label="Document width"
            className="inline-flex h-7 items-center gap-0.5 rounded-md border border-edge bg-panel-2 p-0.5"
          >
            {([
              ["Narrow", false],
              ["Wide", true],
            ] as const).map(([label, wide]) => (
              <button
                key={label}
                type="button"
                onClick={() => setDocWide(wide)}
                aria-pressed={docWide === wide}
                className={`rounded px-2.5 py-0.5 text-xs font-medium transition-colors ${
                  docWide === wide
                    ? "bg-accent/20 text-accent"
                    : "text-ink-dim hover:text-ink"
                }`}
              >
                {label}
              </button>
            ))}
          </div>
          {/* Reveal in the OS file manager — icon only to keep the bar light. */}
          <button
            type="button"
            onClick={() => revealDoc.mutate(meta.id)}
            disabled={revealDoc.isPending}
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink disabled:opacity-50"
            title={revealLabel}
            aria-label={revealLabel}
          >
            <FolderIcon size={16} />
          </button>
          {/* One toggle: a pencil to edit, a magnifying glass to preview (saving
              on the way out when there are changes). Accent-tinted while there
              are unsaved edits, to flag the save the magnifying glass performs. */}
          <button
            type="button"
            onClick={primaryAction}
            disabled={saveDoc.isPending}
            className={`inline-flex h-7 w-7 items-center justify-center rounded-md disabled:opacity-50 ${
              mode === "edit" && dirty
                ? "bg-accent/20 text-accent"
                : "text-ink-dim hover:bg-edge/60 hover:text-ink"
            }`}
            title={primaryTitle}
            aria-label={primaryTitle}
          >
            {saveDoc.isPending ? (
              <Spinner size={14} />
            ) : mode === "edit" ? (
              <SearchIcon size={15} />
            ) : (
              <PencilIcon size={15} />
            )}
          </button>
        </div>
      </header>

      {(saveDoc.isError || patchFrontmatter.isError || revealDoc.isError) && (
        <div className="shrink-0 space-y-2 border-b border-edge p-4">
          {saveDoc.isError && <ErrorNote error={saveDoc.error} />}
          {patchFrontmatter.isError && <ErrorNote error={patchFrontmatter.error} />}
          {revealDoc.isError && <ErrorNote error={revealDoc.error} />}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className={`mx-auto px-8 py-6 ${docWide ? "max-w-none" : "max-w-3xl"}`}>
          <h1 className="selectable mb-5 text-2xl font-semibold text-ink">{docTitle(meta)}</h1>

          {/* A successful patch invalidates the doc query; the refetched mtime
              re-keys this whole pane (see DocDetail), so no local refresh here. */}
          <FrontmatterPanel
            meta={meta}
            disabled={dirty}
            busy={patchFrontmatter.isPending}
            onApply={(patch) => patchFrontmatter.mutate({ patch, baseMtimeMs: mtimeMs })}
          />
          {dirty && (
            <p className="mt-2 text-xs text-warn">
              Save or discard editor changes before patching frontmatter fields.
            </p>
          )}

          <div className="mt-6">
            {mode === "edit" ? (
              <CodeMirror
                value={content}
                minHeight="60vh"
                theme="none"
                extensions={editorExtensions}
                basicSetup={{
                  foldGutter: true,
                  // `codeBase` supplies the themed gutter + syntax highlighting;
                  // disable the defaults so they don't double up / re-color.
                  lineNumbers: false,
                  syntaxHighlighting: false,
                  highlightActiveLine: true,
                }}
                onChange={(value) => {
                  setContent(value);
                  setDirty(value !== seed);
                }}
              />
            ) : (
              <MarkdownPreview body={body} />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
