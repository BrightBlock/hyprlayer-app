import type { TagStyle } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";

// Shared between the workstream code editors and the Library doc editor (heading/link
// rules stay with each caller, since those tags mean different things per surface).
export const CODE_TOKEN_STYLES: TagStyle[] = [
  { tag: [t.keyword, t.modifier, t.operatorKeyword], color: "var(--color-accent)" },
  { tag: [t.comment], color: "var(--color-ink-dim)", fontStyle: "italic" },
  { tag: [t.string, t.special(t.string), t.regexp], color: "var(--color-ok)" },
  { tag: [t.number, t.bool, t.atom], color: "var(--color-warn)" },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--color-brand)" },
  { tag: [t.typeName, t.className, t.namespace], color: "var(--color-brand-soft)" },
  { tag: [t.propertyName, t.attributeName], color: "var(--color-brand-soft)" },
  { tag: [t.tagName], color: "var(--color-accent)" },
  { tag: [t.invalid], color: "var(--color-bad)" },
];
