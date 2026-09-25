
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

export async function langForPath(path: string): Promise<Extension | null> {
  const desc = LanguageDescription.matchFilename(languages, basename(path));
  if (!desc) return null;
  try {
    return await desc.load();
  } catch {
    return null;
  }
}

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

const themeChrome = EditorView.theme({
  "&": {
    backgroundColor: "transparent",
    color: "var(--color-ink)",
    fontSize: "var(--text-meta)",
    height: "100%",
  },
  ".cm-scroller": {
    fontFamily: "var(--font-mono)",
    fontVariantNumeric: "tabular-nums",
    lineHeight: "var(--text-meta--line-height)",
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

export const codeBase: Extension[] = [
  lineNumbers(),
  themeChrome,
  syntaxHighlighting(highlightStyle),
];

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
      if (from >= to) continue;
      for (const [segFrom, segTo] of intraLineRanges(text, from, to)) {
        builder.add(segFrom, segTo, changedTextMark);
      }
    }
  }
  return builder.finish();
}

export const diffWordHighlight: Extension = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    private chunks: readonly Chunk[] | undefined;
    constructor(view: EditorView) {
      this.decorations = buildDiffWordDeco(view.state);
      this.chunks = getChunks(view.state)?.chunks;
    }
    update(update: ViewUpdate) {
      const chunks = getChunks(update.state)?.chunks;
      if (chunks !== this.chunks) {
        this.chunks = chunks;
        this.decorations = buildDiffWordDeco(update.state);
      }
    }
  },
  { decorations: (v) => v.decorations },
);
