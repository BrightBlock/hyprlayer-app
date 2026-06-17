// Shared CodeMirror setup for the read-only file viewer and the diff merge view.
// Everything is bound to the app's CSS theme tokens (the same `--color-*` vars
// index.css defines per theme), so the editor recolors automatically with the
// active theme instead of hardcoding a single dark palette.

import { useEffect, useState } from "react";

import { HighlightStyle, LanguageDescription, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

import { basename } from "../../lib/path";

/** Resolve a CodeMirror language extension for a file path, or null for plain
 *  text. Grammars load on demand from @codemirror/language-data. */
export async function langForPath(path: string): Promise<Extension | null> {
  const desc = LanguageDescription.matchFilename(languages, basename(path));
  if (!desc) return null;
  try {
    return await desc.load();
  } catch {
    return null;
  }
}

/** Load the language grammar for `path` (async, on-demand), null until resolved
 *  or for plain text. Shared by the diff and file-viewer tabs. */
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

/** Editor chrome bound to the theme tokens: transparent surface, app ink, a
 *  panel gutter. Used by both the viewer and the merge view. */
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
  // Merge view: flat translucent add/del boxes. Green matches the header "Ship"
  // button (Tailwind emerald-400, rgb 52 211 153); red stays on the app's bad
  // hue. !important because @codemirror/merge's base theme targets these with a
  // higher-specificity `&light`/`&dark` selector that would otherwise win.
  ".cm-changedLine": { backgroundColor: "rgba(52, 211, 153, 0.20) !important" },
  ".cm-deletedChunk": { backgroundColor: "rgba(239, 83, 80, 0.18) !important" },
  ".cm-changedText": {
    background: "rgba(52, 211, 153, 0.45) !important",
    backgroundImage: "none !important",
  },
  ".cm-deletedChunk .cm-changedText, .cm-deletedText": {
    background: "rgba(239, 83, 80, 0.42) !important",
    backgroundImage: "none !important",
  },
  // The "N unchanged lines" placeholder. @codemirror/merge's base theme paints
  // this with a light gradient (it assumes a light editor unless told otherwise),
  // which reads as a white band on our dark surfaces — force it onto the panel
  // tone with !important so it wins regardless of stylesheet order.
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

/** Token colors derived from theme tokens (keeps highlighting on-palette across
 *  every theme rather than shipping a fixed scheme). */
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

/** Read-only base extensions shared by the viewer and both diff modes. Includes
 *  a line-number gutter — the merge view's `gutter: true` only adds the change
 *  marker gutter, not line numbers. */
export const codeBase: Extension[] = [
  lineNumbers(),
  themeChrome,
  syntaxHighlighting(highlightStyle),
];

/** The full read-only extension set for a CodeMirror view: non-editable, wrapped,
 *  themed base, plus the resolved language grammar (when one loaded). Shared by
 *  the file viewer and both diff modes so they can't drift. */
export function readOnlyExtensions(lang: Extension | null): Extension[] {
  return [
    EditorView.editable.of(false),
    EditorState.readOnly.of(true),
    EditorView.lineWrapping,
    ...codeBase,
    ...(lang ? [lang] : []),
  ];
}
