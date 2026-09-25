import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

import { CODE_TOKEN_STYLES } from "@/lib/codeHighlight";
import { fencedCodeBlocks, fencedCodeTheme } from "./fencedCode";
import { livePreview } from "./livePreview";

const proseTheme = EditorView.theme({
  "&": { backgroundColor: "transparent", color: "var(--color-ink)", fontSize: "var(--text-body-lg)" },
  ".cm-scroller": { fontFamily: "var(--font-sans)", lineHeight: "var(--leading-prose)" },
  "&.cm-focused": { outline: "none" },
  ".cm-content": { padding: "0" },
  ".cm-cursor": { borderLeftColor: "var(--color-accent)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
    backgroundColor: "color-mix(in srgb, var(--color-accent) 25%, transparent)",
  },
});

const markdownHighlight = HighlightStyle.define([
  { tag: t.heading1, fontSize: "var(--text-h1)", fontWeight: "700", letterSpacing: "var(--text-h1--letter-spacing)" },
  { tag: t.heading2, fontSize: "var(--text-h2)", fontWeight: "700" },
  { tag: t.heading3, fontSize: "var(--text-h3)", fontWeight: "700" },
  { tag: t.heading4, fontSize: "var(--text-title)", fontWeight: "700" },
  { tag: t.strong, fontWeight: "700" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: t.monospace, fontFamily: "var(--font-mono)", fontSize: "0.85em" },
  { tag: [t.link, t.url], color: "var(--color-info)", textDecoration: "underline" },
  { tag: t.quote, color: "var(--color-ink-dim)" },
  { tag: t.processingInstruction, color: "var(--color-ink-4)" },
]);

export const docEditorExtensions = [
  proseTheme,
  fencedCodeTheme,
  syntaxHighlighting(markdownHighlight),
  syntaxHighlighting(HighlightStyle.define(CODE_TOKEN_STYLES)),
  EditorView.lineWrapping,
  markdown({ base: markdownLanguage, codeLanguages: languages }),
  livePreview,
  fencedCodeBlocks,
];
