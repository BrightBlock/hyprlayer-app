// Shared CodeMirror setup for the read-only file viewer and diff merge view.
// Bound to the app's `--color-*` theme tokens (defined per theme in index.css)
// so the editor recolors with the active theme rather than hardcoding a palette.

import { useEffect, useState } from "react";

import { HighlightStyle, LanguageDescription, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { type Chunk, getChunks } from "@codemirror/merge";
import { EditorState, type Extension, RangeSetBuilder } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  lineNumbers,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

import { basename } from "@/lib/path";
import { intraLineRanges } from "./diff";

/** Resolve a CodeMirror language extension for a file path (grammar loaded on
 *  demand), or null for plain text. */
export async function langForPath(path: string): Promise<Extension | null> {
  const desc = LanguageDescription.matchFilename(languages, basename(path));
  if (!desc) return null;
  try {
    return await desc.load();
  } catch {
    return null;
  }
}

/** React hook: language grammar for `path`, null until resolved (async,
 *  on-demand) or for plain text. */
export function useLangForPath(path: string): Extension | null {
  const [lang, setLang] = useState<Extension | null>(null);
  useEffect(() => {
    let alive = true;
    setLang(null);
    void langForPath(path).then((ext) => {
      if (alive) setLang(ext ?? null);
    });
    return () => {
      alive = false;
    };
  }, [path]);
  return lang;
}

// Editor chrome bound to the theme tokens; used by the viewer and merge view.
const themeChrome = EditorView.theme({
  "&": {
    backgroundColor: "transparent",
    color: "var(--color-ink)",
    fontSize: "12px",
    height: "100%",
  },
  ".cm-scroller": {
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, monospace",
    lineHeight: "1.5",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-gutters": {
    backgroundColor: "transparent",
    color: "color-mix(in srgb, var(--color-ink-dim) 70%, transparent)",
    border: "none",
  },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 6px 0 8px" },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--color-edge) 30%, transparent)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
    backgroundColor: "color-mix(in srgb, var(--color-accent) 25%, transparent)",
  },
  ".cm-cursor": { borderLeftColor: "var(--color-accent)" },
  // GitHub-style diff highlighting, bound to the themed status hues so the diff
  // recolors per theme. The whole added/removed line gets a light line-level tint
  // (--color-ok additions on the `b` side, --color-bad deletions on the `a` side
  // and the unified deleted widget). A stronger word-level tint
  // (.cm-diffChangedText, emitted by `diffWordHighlight`) marks only the
  // characters that actually changed inside a *modified* line — never a wholly
  // new or removed line. @codemirror/merge's own `highlightChanges` is turned off
  // in DiffTab because it fills whole inserted lines (the double-highlight that
  // made new code hard to read). !important beats merge's higher-specificity
  // `&light`/`&dark` base theme.
  "&.cm-merge-b .cm-changedLine": {
    backgroundColor: "color-mix(in srgb, var(--color-ok) 11%, transparent) !important",
  },
  "&.cm-merge-a .cm-changedLine": {
    backgroundColor: "color-mix(in srgb, var(--color-bad) 11%, transparent) !important",
  },
  ".cm-deletedChunk": { backgroundColor: "color-mix(in srgb, var(--color-bad) 11%, transparent) !important" },
  "&.cm-merge-b .cm-diffChangedText": {
    backgroundColor: "color-mix(in srgb, var(--color-ok) 26%, transparent) !important",
  },
  "&.cm-merge-a .cm-diffChangedText": {
    backgroundColor: "color-mix(in srgb, var(--color-bad) 26%, transparent) !important",
  },
  // "N unchanged lines" placeholder. @codemirror/merge paints this a light
  // gradient (assumes a light editor) → white band on dark surfaces; !important
  // forces it onto the panel tone regardless of stylesheet order.
  ".cm-collapsedLines": {
    padding: "4px 0 4px 12px",
    background: "var(--color-panel-2) !important",
    backgroundImage: "none !important",
    color: "var(--color-ink-dim) !important",
    borderTop: "1px solid var(--color-edge)",
    borderBottom: "1px solid var(--color-edge)",
  },
  ".cm-collapsedLines:hover": {
    background: "var(--color-edge) !important",
    color: "var(--color-ink) !important",
  },
});

// Token colors derived from theme tokens, so highlighting stays on-palette across themes.
const highlightStyle = HighlightStyle.define([
  { tag: [t.keyword, t.modifier, t.operatorKeyword], color: "var(--color-accent)" },
  { tag: [t.comment], color: "var(--color-ink-dim)", fontStyle: "italic" },
  { tag: [t.string, t.special(t.string), t.regexp], color: "var(--color-ok)" },
  { tag: [t.number, t.bool, t.atom], color: "var(--color-warn)" },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--color-brand)" },
  { tag: [t.typeName, t.className, t.namespace], color: "var(--color-brand-soft)" },
  { tag: [t.propertyName, t.attributeName], color: "var(--color-brand-soft)" },
  { tag: [t.tagName], color: "var(--color-accent)" },
  { tag: [t.heading], color: "var(--color-ink)", fontWeight: "bold" },
  { tag: [t.link, t.url], color: "var(--color-brand)", textDecoration: "underline" },
  { tag: [t.invalid], color: "var(--color-bad)" },
]);

/** Read-only base extensions for the viewer and both diff modes. Includes a
 *  line-number gutter (merge view's `gutter: true` adds only change markers). */
export const codeBase: Extension[] = [
  lineNumbers(),
  themeChrome,
  syntaxHighlighting(highlightStyle),
];

/** Full read-only extension set for a CodeMirror view: non-editable, wrapped,
 *  themed base, plus the resolved language grammar. Shared so views can't drift. */
export function readOnlyExtensions(lang: Extension | null): Extension[] {
  return [
    EditorView.editable.of(false),
    EditorState.readOnly.of(true),
    EditorView.lineWrapping,
    ...codeBase,
    ...(lang ? [lang] : []),
  ];
}

const changedTextMark = Decoration.mark({ class: "cm-diffChangedText" });

/** Word-level highlight decorations for a merge editor: `.cm-diffChangedText`
 *  over only the intra-line slices of each change, so wholly-new / wholly-removed
 *  lines keep just their line-level tint (see `intraLineRanges`). Reads the merge
 *  view's own computed chunks via the public `getChunks`; returns nothing in a
 *  plain (non-merge) editor. Exported for testing. */
export function buildDiffWordDeco(state: EditorState): DecorationSet {
  const info = getChunks(state);
  const builder = new RangeSetBuilder<Decoration>();
  if (!info) return builder.finish();
  const isA = info.side === "a";
  const text = state.doc.toString();
  for (const chunk of info.chunks) {
    const base = isA ? chunk.fromA : chunk.fromB;
    for (const change of chunk.changes) {
      const from = base + (isA ? change.fromA : change.fromB);
      const to = base + (isA ? change.toA : change.toB);
      // Empty on this side → a pure insertion/deletion the sibling editor renders.
      if (from >= to) continue;
      for (const [segFrom, segTo] of intraLineRanges(text, from, to)) {
        builder.add(segFrom, segTo, changedTextMark);
      }
    }
  }
  return builder.finish();
}

/** GitHub-style word-level diff highlighting for the merge view in `DiffTab`.
 *  @codemirror/merge's built-in `highlightChanges` tints *every* changed span —
 *  including the whole of a brand-new line — which, with a filled highlight,
 *  double-tints new code and hurts readability. This replaces it: pair with
 *  `highlightChanges: false` and it highlights only the characters that changed
 *  inside a modified line. Safe to include in any read-only view — it no-ops when
 *  the editor has no merge chunks. */
export const diffWordHighlight: Extension = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    private chunks: readonly Chunk[] | undefined;
    constructor(view: EditorView) {
      this.decorations = buildDiffWordDeco(view.state);
      this.chunks = getChunks(view.state)?.chunks;
    }
    update(update: ViewUpdate) {
      // Chunks are computed once for a read-only diff; rebuild only if that
      // identity changes (e.g. they weren't ready at construct time).
      const chunks = getChunks(update.state)?.chunks;
      if (chunks !== this.chunks) {
        this.chunks = chunks;
        this.decorations = buildDiffWordDeco(update.state);
      }
    }
  },
  { decorations: (v) => v.decorations },
);
