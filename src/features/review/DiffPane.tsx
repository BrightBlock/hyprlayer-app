import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { openPath } from "@tauri-apps/plugin-opener";
import { EditorView } from "@codemirror/view";

import { DiffView, useDiffMode, type DiffMode, type DiffViews } from "@/components/DiffView";
import { Icon } from "@/components/Icon";
import { Button, EmptyState, ErrorNote, IconButton, SegmentedControl, Spinner } from "@/components/ui";
import type { ReviewFile, ReviewNote, ReviewSide, WorkstreamView } from "@/lib/bindings";
import { useLangForPath } from "@/lib/codeView";
import { CHANGE_GLYPH } from "@/lib/domain/review";
import { plural } from "@/lib/format";
import { log } from "@/lib/log";
import { basename, dirname } from "@/lib/path";
import { useConnectionStatus, useEditorTargets, useReviewFileSides } from "@/lib/queries";

import { CommentComposer } from "./CommentComposer";
import { CommentLayer, slotKey } from "./commentGutter";
import { CommentThread } from "./CommentThread";
import { KindTag, NoteConversation } from "./NoteCard";
import { type Slot, ThreadPortals } from "./ThreadPortals";
import { type Thread, threadSize } from "./threads";

const MODE_OPTIONS = [
  { value: "unified", label: "Unified" },
  { value: "split", label: "Side by side" },
] as const;

function useDefaultEditor() {
  const targets = useEditorTargets();
  const remote = useConnectionStatus().data?.host != null;
  const editor = useMemo(() => {
    const apps = targets.data ?? [];
    return apps.find((a) => a.kind === "editor") ?? apps.find((a) => a.id !== "files");
  }, [targets.data]);
  // A remote server's path doesn't exist on this machine.
  return remote ? undefined : editor;
}

function scrollToLine(view: EditorView, line: number) {
  const at = view.state.doc.line(Math.min(line, view.state.doc.lines)).from;
  view.dispatch({ effects: EditorView.scrollIntoView(at, { y: "center" }) });
}

// CodeMirror breaks lines at \r\n, \r and \n.
function lineCount(text: string): number {
  let n = 1;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    if (c === 13 && text.charCodeAt(i + 1) === 10) i++;
    if (c === 10 || c === 13) n++;
  }
  return n;
}

type Composer = { path: string; side: ReviewSide; line: number; anchorText: string; noteId?: string };

function LineContext({ side, line, text }: { side: ReviewSide; line: number; text: string }) {
  return (
    <div className="truncate font-mono text-meta text-ink-dim" title={text}>
      <span className={side === "original" ? "text-diff-del" : "text-ink-4"}>
        {side === "original" ? "−" : ""}
        {line}
      </span>{" "}
      {text}
    </div>
  );
}

export function DiffPane({
  view,
  file,
  index,
  total,
  viewed,
  next,
  allViewed,
  line,
  threads,
  notes,
  agentLabel,
  onToggleViewed,
  onPrev,
  onNext,
  onMarkViewedNext,
}: {
  view: WorkstreamView;
  file: ReviewFile;
  index: number;
  total: number;
  viewed: boolean;
  next: ReviewFile | null;
  allViewed: boolean;
  line?: number;
  threads: Thread[];
  notes: ReviewNote[];
  agentLabel: string;
  onToggleViewed: () => void;
  onPrev: () => void;
  onNext: () => void;
  onMarkViewedNext: () => void;
}) {
  // Binary files are known from the list; don't pull their sides at all.
  const sides = useReviewFileSides(view.item.id, file.path, !file.binary);
  const [mode, setMode] = useDiffMode();
  const lang = useLangForPath(file.path);
  const editor = useDefaultEditor();
  const scrolledTo = useRef<string | null>(null);
  const [layer] = useState(() => new CommentLayer());
  const extensions = useMemo(
    () => ({ original: layer.extension("original"), modified: layer.extension("modified") }),
    [layer],
  );
  const [opened, setOpened] = useState<Composer | null>(null);
  const composer = opened?.path === file.path ? opened : null;
  const wsId = view.item.id;

  useEffect(() => {
    layer.onAdd = (side, n) => {
      const doc = layer.view(side)?.state.doc;
      const anchorText = doc && n <= doc.lines ? doc.line(n).text : "";
      setOpened({ path: file.path, side, line: n, anchorText });
    };
  }, [layer, file.path]);

  const data = sides.data;
  const docLines = useMemo(
    () => data && { original: lineCount(data.original), modified: lineCount(data.modified) },
    [data],
  );
  const oneSided = !!data && (!data.originalExists || !data.modifiedExists);
  const effectiveMode: DiffMode = oneSided ? "unified" : mode;
  const glyph = CHANGE_GLYPH[file.change];
  const dir = dirname(file.path);
  const openNotes = notes.filter((n) => !n.resolved);
  const outOfScope = openNotes.some((n) => n.kind === "outside_plan_scope");

  // Unified view has no Original editor, so threads anchored there list above the diff, as does
  // everything on a file shown without one.
  const noDiff = file.binary || !!data?.binary || !!data?.tooLarge;
  const inline = (side: ReviewSide) => !noDiff && (side === "modified" || effectiveMode === "split");
  // An outdated item sits where its line was, within what is left of the file.
  const placeAt = (side: ReviewSide, current: number | null, line: number) =>
    current ?? Math.max(1, Math.min(line, docLines?.[side] ?? line));
  const above: ReactNode[] = [];
  const bySlot = new Map<string, { side: ReviewSide; line: number; items: ReactNode[] }>();
  const at = (side: ReviewSide, n: number) => {
    const key = slotKey(side, n);
    let slot = bySlot.get(key);
    if (!slot) {
      slot = { side, line: n, items: [] };
      bySlot.set(key, slot);
    }
    return slot.items;
  };
  // A note's replies render inside its box, so the exchange reads as one conversation.
  const answers = new Map<string, Thread[]>();
  const noteIds = new Set(notes.map((n) => n.id));
  const loose: Thread[] = [];
  for (const t of threads) {
    const id = t.root.replyToNoteId;
    if (id && noteIds.has(id)) answers.set(id, [...(answers.get(id) ?? []), t]);
    else loose.push(t);
  }
  for (const n of notes) {
    const replies = answers.get(n.id) ?? [];
    const replying = composer?.noteId === n.id;
    // An outdated note's reply stays anchored to the note's own line: the server re-anchors it
    // like any comment, so it goes outdated with the note and follows it if the line returns.
    const conversation = (
      <NoteConversation
        key={n.id}
        note={n}
        workstreamId={wsId}
        replyCount={replies.reduce((sum, t) => sum + threadSize(t), 0)}
        outdated={n.currentLine == null}
        composer={
          replying && (
            <CommentComposer
              request={{
                workstreamId: wsId,
                path: file.path,
                side: "modified",
                line: n.currentLine ?? n.line,
                anchorText: "",
                parentId: null,
                replyToNoteId: n.id,
              }}
              agentLabel={agentLabel}
              onDone={() => setOpened(null)}
            />
          )
        }
        // The first reply opens the thread; later ones go through its own Reply.
        onReply={
          replies.length === 0 && !replying
            ? () => setOpened({ path: file.path, side: "modified", line: n.currentLine ?? n.line, anchorText: "", noteId: n.id })
            : undefined
        }
      >
        {replies.map((t) => (
          <CommentThread key={t.root.id} thread={t} workstreamId={wsId} agentLabel={agentLabel} embedded />
        ))}
      </NoteConversation>
    );
    if (inline("modified")) at("modified", placeAt("modified", n.currentLine, n.line)).push(conversation);
    else above.push(conversation);
  }
  for (const t of loose) {
    const { root } = t;
    const outdated = root.currentLine == null;
    const context = <LineContext side={root.side} line={root.currentLine ?? root.line} text={root.anchorText} />;
    const render = (withContext: boolean) => (
      <CommentThread
        key={root.id}
        thread={t}
        workstreamId={wsId}
        agentLabel={agentLabel}
        context={withContext ? context : undefined}
        outdated={outdated}
      />
    );
    // Inline, an outdated thread's spot no longer shows the text it was left on.
    if (inline(root.side)) at(root.side, placeAt(root.side, root.currentLine, root.line)).push(render(outdated));
    else above.push(render(true));
  }
  const lineComposer = composer && !composer.noteId && inline(composer.side) ? composer : null;
  if (lineComposer) {
    at(lineComposer.side, lineComposer.line).push(
      <CommentComposer
        key="composer"
        request={{
          workstreamId: wsId,
          path: file.path,
          side: lineComposer.side,
          line: lineComposer.line,
          anchorText: lineComposer.anchorText,
          parentId: null,
        }}
        agentLabel={agentLabel}
        onDone={() => setOpened(null)}
      />,
    );
  }
  const slots: Slot[] = [...bySlot.values()].map((s) => ({
    side: s.side,
    line: s.line,
    content: <div className="space-y-2">{s.items}</div>,
  }));

  const onViews = (v: DiffViews) => {
    const key = `${file.path}:${line}`;
    if (!line || scrolledTo.current === key) return;
    scrolledTo.current = key;
    scrollToLine(v.modified, line);
  };

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-edge px-4">
        <span className={`w-3 shrink-0 text-center text-meta font-semibold ${glyph.cls}`} title={glyph.label}>
          {glyph.ch}
        </span>
        <span className="min-w-0 flex-1 truncate text-meta" title={file.path}>
          {file.oldPath ? (
            <span className="text-ink">
              {file.oldPath} → {file.path}
            </span>
          ) : (
            <>
              {dir && <span className="text-ink-4">{dir}/</span>}
              <span className="font-semibold text-ink">{basename(file.path)}</span>
            </>
          )}
        </span>
        {!file.binary && (
          <span className="shrink-0 text-meta tabular-nums">
            <span className="text-diff-add">+{file.additions}</span>{" "}
            <span className="text-diff-del">−{file.deletions}</span>
          </span>
        )}
        {openNotes.length > 0 && (
          <span className="inline-flex h-5 shrink-0 items-center whitespace-nowrap rounded-control border border-edge px-2 text-meta font-semibold normal-case text-ink-2">
            {plural(openNotes.length, "note")}
          </span>
        )}
        {outOfScope && <KindTag kind="outside_plan_scope" />}
        <span className="shrink-0 text-meta tabular-nums text-ink-4">
          {index + 1} of {total}
        </span>
        <span className="flex shrink-0 items-center gap-1">
          <IconButton className="h-6 w-6" onClick={onPrev} disabled={total < 2} title="Previous file" aria-label="Previous file">
            <Icon name="chevronLeft" size="sm" />
          </IconButton>
          <IconButton className="h-6 w-6" onClick={onNext} disabled={total < 2} title="Next file" aria-label="Next file">
            <Icon name="chevronRight" size="sm" />
          </IconButton>
        </span>
        {editor && file.change !== "deleted" && (
          <Button
            size="sm"
            onClick={() => {
              const abs = `${view.item.worktreePath}/${file.path}`;
              const opened = editor.openWith ? openPath(abs, editor.openWith) : openPath(abs);
              void opened.catch((e) => log.error("review", `opening in ${editor.name} failed`, e));
            }}
          >
            Open
          </Button>
        )}
        <label className="flex h-6 shrink-0 cursor-pointer items-center gap-1.5 rounded-control border border-edge pl-2 pr-2.5 text-ui text-ink-2 hover:bg-edge/50">
          <input
            type="checkbox"
            className="h-3.5 w-3.5 shrink-0 cursor-pointer accent-accent"
            checked={viewed}
            onChange={onToggleViewed}
          />
          Viewed
        </label>
        <SegmentedControl
          aria-label="Diff layout"
          value={effectiveMode}
          options={MODE_OPTIONS}
          onChange={setMode}
          disabled={oneSided}
        />
      </div>

      {above.length > 0 && (
        <div className="max-h-[40%] shrink-0 space-y-2 overflow-y-auto border-b border-edge bg-panel-2/40 px-4 pb-3 pt-2.5 [&>*]:max-w-[min(520px,75%)]">
          {above}
        </div>
      )}

      <div className="flex min-h-0 flex-1 flex-col">
        {file.binary ? (
          <EmptyState title="Binary file" />
        ) : sides.isLoading ? (
          <div className="flex flex-1 items-center justify-center">
            <Spinner />
          </div>
        ) : sides.isError ? (
          <div className="p-4">
            <ErrorNote error={sides.error} />
          </div>
        ) : data?.binary ? (
          <EmptyState title="Binary file" />
        ) : data?.tooLarge ? (
          <EmptyState title="File too large to diff" />
        ) : data ? (
          <>
            <DiffView
              original={data.original}
              modified={data.modified}
              mode={effectiveMode}
              lang={lang}
              extensions={extensions}
              onViews={onViews}
            />
            <ThreadPortals
              layer={layer}
              slots={slots}
              composer={lineComposer}
            />
          </>
        ) : null}
      </div>

      <div className="flex h-12 shrink-0 items-center justify-end gap-2 border-t border-edge px-4">
        {allViewed ? (
          <span className="text-ui text-ink-dim">All {plural(total, "file")} viewed.</span>
        ) : (
          next && (
            <Button variant="primary" onClick={onMarkViewedNext}>
              Mark viewed, next: {basename(next.path)}
            </Button>
          )
        )}
      </div>
    </div>
  );
}
