// Shared CodeMirror setup for the read-only file viewer and diff merge view.
// Bound to the app's `--color-*` theme tokens (defined per theme in index.css)
// so the editor recolors with the active theme rather than hardcoding a palette.

import { useEffect, useState } from "react";

import { HighlightStyle, LanguageDescription, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

import { basename } from "@/lib/path";

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
  // Merge-view add/del boxes bound to the themed status hues (--color-ok adds,
  // --color-bad dels) so the diff recolors per theme, matching the unified diff
  // (see PATCH_LINE_CLASSES in diff.ts). Fill kept light so syntax colors stay
  // legible; word-level highlight a touch stronger. !important to beat
  // @codemirror/merge's higher-specificity `&light`/`&dark` base theme.
  ".cm-changedLine": { backgroundColor: "color-mix(in srgb, var(--color-ok) 11%, transparent) !important" },
  ".cm-deletedChunk": { backgroundColor: "color-mix(in srgb, var(--color-bad) 11%, transparent) !important" },
  ".cm-changedText": {
    background: "color-mix(in srgb, var(--color-ok) 30%, transparent) !important",
    backgroundImage: "none !important",
  },
  ".cm-deletedChunk .cm-changedText, .cm-deletedText": {
    background: "color-mix(in srgb, var(--color-bad) 30%, transparent) !important",
    backgroundImage: "none !important",
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
