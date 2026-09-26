import type { EditorView } from "@codemirror/view";
import { useRouterState } from "@tanstack/react-router";
import CodeMirror from "@uiw/react-codemirror";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { Button, ConfirmDialog, ErrorNote, PageError, PageSpinner, SaveStatus } from "@/components/ui";
import { Icon } from "@/components/Icon";
import type { DocContent } from "@/lib/bindings";
import { useDoc, useRevealDoc } from "@/lib/queries";
import { clearActiveFileSave, setActiveFileSave } from "@/lib/fileSave";
import { IS_MACOS } from "@/lib/platform";
import { startEdgeDrag } from "@/lib/resize";
import { useReducedMotion } from "@/lib/useReducedMotion";
import { DOC_RAIL_MAX, DOC_RAIL_MIN, useUiStore } from "@/lib/uiStore";
import { activeHeading, createActiveHeading, jumpToElement, jumpToHeading } from "./editor/contentsNav";
import { docEditorExtensions } from "./editor/docEditor";
import { extractHeadings } from "./editor/headings";
import { recalledScroll, rememberScroll } from "./docScroll";
import { DocRail } from "./DocRail";
import { MarkdownPreview } from "./MarkdownPreview";
import { useDocSave } from "./useDocSave";

const HEADINGS_DEBOUNCE_MS = 150;

type Props = { id: string };

type Anchor = { line: number; offset: number };

const lineOf = (el: Element) => Number(el.getAttribute("data-line"));

export function DocDetail({ id }: Props) {
  const doc = useDoc(id);
  if (doc.isLoading) return <PageSpinner />;
  if (doc.isError) return <PageError error={doc.error} />;
  if (!doc.data) return null;
  return <DocBody key={id} id={id} doc={doc.data} />;
}

type BodyProps = Props & { doc: DocContent };

function DocBody({ id, doc }: BodyProps) {
  const revealDoc = useRevealDoc();
  const docWide = useUiStore((s) => s.docWide);
  const setDocWide = useUiStore((s) => s.setDocWide);
  const { meta } = doc;

  const {
    body,
    setBody,
    draft,
    setField,
    raw,
    setRaw,
    dirty,
    saving,
    justSaved,
    save,
    conflict,
    reloadFromDisk,
    error,
    blocker,
  } = useDocSave(id, doc);

  const docRailOpen = useUiStore((s) => s.docRailOpen);
  const docRailWidth = useUiStore((s) => s.docRailWidth);
  const setDocRailWidth = useUiStore((s) => s.setDocRailWidth);
  const reducedMotion = useReducedMotion();

  const [editing, setEditing] = useState(false);
  const [view, setView] = useState<EditorView | null>(null);
  const [headings, setHeadings] = useState(() => extractHeadings(body));
  const headingsBodyRef = useRef(body);
  const [active] = useState(createActiveHeading);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const previewRef = useRef<HTMLDivElement | null>(null);
  const scrollRafRef = useRef<number | null>(null);
  const editAnchorRef = useRef<Anchor | null>(null);
  const previewAnchorRef = useRef<Anchor | null>(null);
  // Scrollspy would walk the highlight through every heading a jump passes.
  const jumpingRef = useRef(false);

  // Each history entry gets its own remembered scroll position, restored on Back.
  const historyKey = useRouterState({
    select: (s) => s.location.state.__TSR_key ?? String(s.location.state.__TSR_index),
  });
  const historyKeyRef = useRef(historyKey);
  historyKeyRef.current = historyKey;

  useEffect(() => {
    if (body === headingsBodyRef.current) return;
    const timer = window.setTimeout(() => {
      headingsBodyRef.current = body;
      setHeadings(extractHeadings(body));
    }, HEADINGS_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [body]);

  useEffect(
    () => () => {
      if (scrollRafRef.current) cancelAnimationFrame(scrollRafRef.current);
    },
    [],
  );

  useLayoutEffect(() => {
    const scroller = scrollerRef.current;
    if (scroller) scroller.scrollTop = recalledScroll(historyKeyRef.current) ?? 0;
  }, []);

  const headingEl = useCallback(
    (heading: { line: number }) =>
      previewRef.current?.querySelector(`:is(h2, h3, h4)[data-line="${heading.line}"]`) ?? null,
    [],
  );

  const enterEdit = (target: Element | null) => {
    const scroller = scrollerRef.current;
    const preview = previewRef.current;
    if (!scroller || !preview) return;
    const top = scroller.getBoundingClientRect().top;
    const block =
      target?.closest("[data-line]") ??
      [...preview.querySelectorAll("[data-line]")].find((b) => b.getBoundingClientRect().bottom > top);
    editAnchorRef.current = block ? { line: lineOf(block), offset: block.getBoundingClientRect().top - top } : null;
    setEditing(true);
  };

  const exitEdit = () => {
    const scroller = scrollerRef.current;
    if (view && scroller) {
      const top = scroller.getBoundingClientRect().top;
      const block = view.lineBlockAtHeight(top - view.documentTop);
      previewAnchorRef.current = {
        line: view.state.doc.lineAt(block.from).number,
        offset: view.documentTop + block.top - top,
      };
    }
    setView(null);
    setEditing(false);
  };

  // Keeps the line that was at the top of the editor at the same height in the rendered doc.
  useLayoutEffect(() => {
    const anchor = previewAnchorRef.current;
    const scroller = scrollerRef.current;
    const preview = previewRef.current;
    if (editing || !anchor || !scroller || !preview) return;
    previewAnchorRef.current = null;
    let target: Element | null = null;
    for (const block of preview.querySelectorAll("[data-line]")) {
      if (lineOf(block) > anchor.line) break;
      target = block;
    }
    if (target) {
      scroller.scrollTop += target.getBoundingClientRect().top - scroller.getBoundingClientRect().top - anchor.offset;
    }
  }, [editing]);

  const handleScroll = useCallback(() => {
    if (scrollRafRef.current) return;
    scrollRafRef.current = requestAnimationFrame(() => {
      scrollRafRef.current = null;
      const scroller = scrollerRef.current;
      if (!scroller) return;
      rememberScroll(historyKeyRef.current, scroller.scrollTop);
      if (jumpingRef.current) return;
      if (editing && view) {
        const topOf = (i: number) =>
          view.documentTop + view.lineBlockAt(Math.min(headings[i].from, view.state.doc.length)).top;
        active.set(activeHeading(headings.length, topOf, scroller));
      } else if (!editing) {
        const topOf = (i: number) => headingEl(headings[i])?.getBoundingClientRect().top ?? -Infinity;
        active.set(activeHeading(headings.length, topOf, scroller));
      }
    });
  }, [editing, view, headings, headingEl, active]);

  const handleJumpToHeading = useCallback(
    (index: number) => {
      const heading = headings[index];
      const scroller = scrollerRef.current;
      if (!heading || !scroller) return;
      const landed = () => {
        jumpingRef.current = false;
      };
      if (editing && view) {
        active.set(index);
        jumpingRef.current = true;
        jumpToHeading(view, scroller, heading.from, !reducedMotion, landed);
      } else if (!editing) {
        const el = headingEl(heading);
        if (!el) return;
        active.set(index);
        jumpingRef.current = true;
        jumpToElement(el, scroller, !reducedMotion, landed);
      }
    },
    [editing, headings, view, reducedMotion, headingEl, active],
  );

  const saveRef = useRef<() => void>(() => {});
  saveRef.current = () => {
    void save();
  };

  const tokenRef = useRef<object>({});
  const registerActive = () => setActiveFileSave(tokenRef.current, () => saveRef.current());
  useEffect(() => {
    const token = tokenRef.current;
    setActiveFileSave(token, () => saveRef.current());
    return () => clearActiveFileSave(token);
  }, []);

  const revealLabel = IS_MACOS ? "Reveal in Finder" : "Show in folder";

  return (
    <div
      className="flex h-full min-h-0 flex-col"
      onFocus={registerActive}
      onKeyDown={(e) => {
        if (!IS_MACOS && (e.metaKey || e.ctrlKey) && e.key === "s") {
          e.preventDefault();
          saveRef.current();
        }
      }}
    >
      <header className="flex h-11 shrink-0 items-center justify-between gap-3 border-b border-edge bg-panel px-6">
        <div className="flex min-w-0 items-center gap-2">
          <span className="selectable truncate text-meta text-ink-dim">{meta.relPath}</span>
        </div>
        <div className="flex shrink-0 items-center gap-2">
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
                className={`rounded px-2.5 py-0.5 text-ui font-medium transition-colors ${
                  docWide === wide
                    ? "bg-accent/20 text-accent"
                    : "text-ink-dim hover:text-ink"
                }`}
              >
                {label}
              </button>
            ))}
          </div>
          <button
            type="button"
            onClick={() => revealDoc.mutate(meta.id)}
            disabled={revealDoc.isPending}
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink disabled:opacity-50"
            title={revealLabel}
            aria-label={revealLabel}
          >
            <Icon name="folder" />
          </button>
          <Button
            size="sm"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => (editing ? exitEdit() : enterEdit(null))}
          >
            {editing ? "Done" : "Edit"}
          </Button>
          <SaveStatus saving={saving} dirty={dirty} justSaved={justSaved} readOnly={false} />
          <Button size="sm" variant="primary" disabled={!dirty} busy={saving} onClick={() => void save()}>
            Save
          </Button>
        </div>
      </header>

      {(error != null || revealDoc.isError) && (
        <div className="shrink-0 space-y-2 border-b border-edge p-4">
          {error != null && (
            <div className="space-y-2">
              <ErrorNote error={error} />
              {conflict && <Button onClick={() => void reloadFromDisk()}>Reload from disk</Button>}
            </div>
          )}
          {revealDoc.isError && (
            <ErrorNote error={revealDoc.error} onDismiss={() => revealDoc.reset()} />
          )}
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        <div ref={scrollerRef} className="min-w-0 flex-1 overflow-y-auto" onScroll={handleScroll}>
          <div className={`mx-auto px-8 py-6 ${docWide ? "max-w-none" : "max-w-[600px]"}`}>
            {editing ? (
              <div
                onBlur={(e) => {
                  // Leaving the window blurs the editor too; only a click elsewhere in the app ends editing.
                  if (!e.currentTarget.contains(e.relatedTarget) && document.hasFocus()) exitEdit();
                }}
                onKeyDown={(e) => {
                  if (e.key === "Escape" && !e.defaultPrevented) exitEdit();
                }}
              >
                <CodeMirror
                  value={body}
                  minHeight="60vh"
                  theme="none"
                  extensions={docEditorExtensions}
                  basicSetup={{
                    lineNumbers: false,
                    foldGutter: false,
                    highlightActiveLine: false,
                    syntaxHighlighting: false,
                  }}
                  onChange={setBody}
                  onCreateEditor={(v) => {
                    setView(v);
                    const anchor = editAnchorRef.current;
                    editAnchorRef.current = null;
                    const scroller = scrollerRef.current;
                    if (!anchor || !scroller) return v.focus();
                    const pos = v.state.doc.line(Math.min(anchor.line, v.state.doc.lines)).from;
                    jumpToHeading(v, scroller, pos, false, () => {}, Math.max(0, anchor.offset));
                  }}
                />
              </div>
            ) : (
              <div
                ref={previewRef}
                className="min-h-[60vh] cursor-text"
                onClick={(e) => {
                  const target = e.target as Element;
                  if (target.closest("a, button") || !document.getSelection()?.isCollapsed) return;
                  enterEdit(target);
                }}
              >
                {body.trim() ? (
                  <MarkdownPreview body={body} />
                ) : (
                  <p className="text-body-lg text-ink-4">Empty document. Click to start writing.</p>
                )}
              </div>
            )}
          </div>
        </div>
        {docRailOpen && (
          <>
            <div
              role="separator"
              aria-orientation="vertical"
              aria-label="Resize panels"
              className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
              onPointerDown={(e) =>
                startEdgeDrag(e, {
                  startWidth: docRailWidth,
                  min: DOC_RAIL_MIN,
                  max: DOC_RAIL_MAX,
                  direction: -1,
                  onResize: setDocRailWidth,
                })
              }
            />
            <DocRail
              doc={doc}
              draft={draft}
              onChange={setField}
              raw={raw}
              setRaw={setRaw}
              disabled={saving}
              headings={headings}
              active={active}
              onJumpToHeading={handleJumpToHeading}
              width={docRailWidth}
            />
          </>
        )}
      </div>

      <ConfirmDialog
        open={blocker.status === "blocked"}
        onClose={() => blocker.reset?.()}
        title="Unsaved changes"
        confirmLabel="Save"
        onConfirm={async () => {
          if (await save()) blocker.proceed?.();
        }}
        extraActions={
          <Button disabled={saving} onClick={() => blocker.proceed?.()}>
            Discard
          </Button>
        }
        busy={saving}
        error={error}
      >
        <p className="text-body leading-prose text-ink-dim">This document has unsaved changes.</p>
      </ConfirmDialog>
    </div>
  );
}
