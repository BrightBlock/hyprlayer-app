import { type Chunk, getChunks, mergeViewSiblings, uncollapseUnchanged } from "@codemirror/merge";
import { type EditorState, type Extension, RangeSetBuilder, StateEffect, StateField } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  gutter,
  GutterMarker,
  ViewPlugin,
  WidgetType,
} from "@codemirror/view";

import type { ReviewSide } from "@/lib/bindings";

export const setThreads = StateEffect.define<readonly number[]>();
export const openComposer = StateEffect.define<number | null>();
const setHover = StateEffect.define<number | null>();

type Slots = { lines: readonly number[]; composer: number | null };

const NO_SLOTS: Slots = { lines: [], composer: null };
const SIDES: readonly ReviewSide[] = ["original", "modified"];

export function slotKey(side: ReviewSide, line: number): string {
  return `${side}:${line}`;
}

class SlotWidget extends WidgetType {
  constructor(
    readonly layer: CommentLayer,
    readonly side: ReviewSide,
    readonly line: number,
  ) {
    super();
  }
  // Same key, same DOM: thread data changes never touch the widget.
  eq(other: SlotWidget) {
    return other.layer === this.layer && other.side === this.side && other.line === this.line;
  }
  toDOM() {
    return this.layer.container(this.side, this.line);
  }
}

class AddMarker extends GutterMarker {
  toDOM() {
    const el = document.createElement("span");
    el.className =
      "flex h-4 w-4 cursor-pointer items-center justify-center rounded-control bg-accent font-semibold text-accent-ink";
    el.textContent = "+";
    return el;
  }
}

const addMarker = new AddMarker();

class Spacer extends GutterMarker {
  toDOM() {
    const el = document.createElement("span");
    el.className = "block w-4";
    return el;
  }
}

const hoverField = StateField.define<number | null>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setHover)) value = e.value;
    return value;
  },
});

function sameLines(a: readonly number[], b: readonly number[]): boolean {
  return a.length === b.length && a.every((n, i) => n === b[i]);
}

// Mirrors @codemirror/merge's own mapping, which its collapse widget uses to open both sides.
function mapAcross(pos: number, chunks: readonly Chunk[], isA: boolean): number {
  let ours = 0;
  let theirs = 0;
  for (const c of chunks) {
    if ((isA ? c.fromA : c.fromB) >= pos) break;
    [ours, theirs] = isA ? [c.toA, c.toB] : [c.toB, c.toA];
  }
  return theirs + (pos - ours);
}

// One per diff pane. Owns the `side:line` widget containers React portals into, so the
// editor is never rebuilt for thread changes.
export class CommentLayer {
  onAdd: (side: ReviewSide, line: number) => void = () => {};
  private views: Record<ReviewSide, EditorView | null> = { original: null, modified: null };
  private wanted: Record<ReviewSide, Slots> = { original: NO_SLOTS, modified: NO_SLOTS };
  private containers = new Map<string, HTMLElement>();
  private extensions = new Map<ReviewSide, Extension>();
  private fields = new Map<ReviewSide, StateField<{ slots: Slots; deco: DecorationSet }>>();
  private listeners = new Set<() => void>();
  private version = 0;

  subscribe = (fn: () => void) => {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  };

  getVersion = () => this.version;

  view(side: ReviewSide): EditorView | null {
    return this.views[side];
  }

  container(side: ReviewSide, line: number): HTMLElement {
    const key = slotKey(side, line);
    let el = this.containers.get(key);
    if (!el) {
      el = document.createElement("div");
      el.className = "cm-review-slot max-w-[min(520px,75%)] whitespace-normal pb-3 pl-2 pr-4 pt-2.5 font-sans";
      this.containers.set(key, el);
    }
    return el;
  }

  // Set during render; a view built later starts from it.
  want(side: ReviewSide, slots: Slots): void {
    const prev = this.wanted[side];
    if (prev.composer === slots.composer && sameLines(prev.lines, slots.lines)) return;
    this.wanted[side] = slots;
  }

  flush(): void {
    for (const side of SIDES) {
      const view = this.views[side];
      const field = this.fields.get(side);
      if (!view || !field) continue;
      const want = this.wanted[side];
      const has = view.state.field(field).slots;
      const effects: StateEffect<unknown>[] = [];
      if (!sameLines(has.lines, want.lines)) effects.push(setThreads.of(want.lines));
      if (has.composer !== want.composer) effects.push(openComposer.of(want.composer));
      if (effects.length) view.dispatch({ effects });
      this.expand(view, want);
    }
  }

  extension(side: ReviewSide): Extension {
    let ext = this.extensions.get(side);
    if (!ext) {
      ext = this.build(side);
      this.extensions.set(side, ext);
    }
    return ext;
  }

  private emit() {
    this.version++;
    for (const fn of this.listeners) fn();
  }

  private attach(side: ReviewSide, view: EditorView) {
    this.views[side] = view;
    this.emit();
  }

  private detach(side: ReviewSide, view: EditorView) {
    if (this.views[side] !== view) return;
    this.views[side] = null;
    for (const key of [...this.containers.keys()]) {
      if (key.startsWith(`${side}:`)) this.containers.delete(key);
    }
    this.emit();
  }

  // collapseUnchanged hides a widget on an unchanged line; open the range so notes there show.
  private expand(view: EditorView, slots: Slots) {
    const doc = view.state.doc;
    const lines = slots.composer == null ? slots.lines : [...slots.lines, slots.composer];
    const starts = new Set<number>();
    for (const n of lines) {
      if (n < 1 || n > doc.lines) continue;
      const line = doc.line(n);
      const block = view.lineBlockAt(line.from);
      if (block.from !== line.from || block.to !== line.to) starts.add(block.from);
    }
    if (starts.size === 0) return;
    view.dispatch({ effects: [...starts].map((pos) => uncollapseUnchanged.of(pos)) });
    const pair = mergeViewSiblings(view);
    const chunks = getChunks(view.state);
    if (!pair || !chunks) return;
    const isA = pair.a === view;
    const other = isA ? pair.b : pair.a;
    other.dispatch({
      effects: [...starts].map((pos) => uncollapseUnchanged.of(mapAcross(pos, chunks.chunks, isA))),
    });
  }

  private build(side: ReviewSide): Extension {
    const decorate = (state: EditorState, slots: Slots) => {
      const lines = new Set(slots.lines);
      if (slots.composer != null) lines.add(slots.composer);
      const builder = new RangeSetBuilder<Decoration>();
      for (const n of [...lines].sort((a, b) => a - b)) {
        if (n < 1 || n > state.doc.lines) continue;
        const at = state.doc.line(n).to;
        builder.add(
          at,
          at,
          Decoration.widget({ widget: new SlotWidget(this, side, n), block: true, side: 1 }),
        );
      }
      return builder.finish();
    };

    const field = StateField.define<{ slots: Slots; deco: DecorationSet }>({
      create: (state) => {
        const slots = this.wanted[side];
        return { slots, deco: decorate(state, slots) };
      },
      update(value, tr) {
        let slots = value.slots;
        for (const e of tr.effects) {
          if (e.is(setThreads)) slots = { ...slots, lines: e.value };
          else if (e.is(openComposer)) slots = { ...slots, composer: e.value };
        }
        return slots === value.slots ? value : { slots, deco: decorate(tr.state, slots) };
      },
      provide: (f) => EditorView.decorations.from(f, (v) => v.deco),
    });
    this.fields.set(side, field);

    const lineOf = (view: EditorView, from: number) => view.state.doc.lineAt(from).number;

    const hover = ViewPlugin.define((view) => {
      this.attach(side, view);
      let current: number | null = null;
      const set = (line: number | null) => {
        if (line === current) return;
        current = line;
        view.dispatch({ effects: setHover.of(line) });
      };
      // Threads, unified deleted chunks and collapsed stubs have no line of their own to comment on.
      const move = (e: MouseEvent) => {
        const target = e.target instanceof Element ? e.target : null;
        if (target?.closest(".cm-review-slot, .cm-deletedChunk, .cm-collapsedLines")) return set(null);
        set(lineOf(view, view.lineBlockAtHeight(e.clientY - view.documentTop).from));
      };
      const leave = () => set(null);
      view.dom.addEventListener("mousemove", move);
      view.dom.addEventListener("mouseleave", leave);
      return {
        destroy: () => {
          view.dom.removeEventListener("mousemove", move);
          view.dom.removeEventListener("mouseleave", leave);
          this.detach(side, view);
        },
      };
    });

    return [
      field,
      hoverField,
      hover,
      gutter({
        class: "cm-review-gutter",
        lineMarker: (view, block) =>
          view.state.field(hoverField) === lineOf(view, block.from) ? addMarker : null,
        lineMarkerChange: (u) => u.transactions.some((tr) => tr.effects.some((e) => e.is(setHover))),
        initialSpacer: () => new Spacer(),
        domEventHandlers: {
          // Only where the "+" shows, so a deleted chunk's gutter row never comments on the next line.
          mousedown: (view, block) => {
            const line = lineOf(view, block.from);
            if (view.state.field(hoverField) !== line) return false;
            this.onAdd(side, line);
            return true;
          },
        },
      }),
    ];
  }
}
