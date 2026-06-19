// Lazy-loaded Shiki highlighter for chat-pane fenced code blocks. The custom
// theme below maps TextMate scopes to the same `--color-*` CSS variables that
// codeView.ts uses for the file viewer and diff view, so chat code blocks
// recolor with the active theme without any per-theme palette duplication.
//
// Shiki itself is dynamic-imported on first use so the (~100 KB) core only
// loads when the user actually sees an assistant message with code in it.
// Grammars load on demand via loadLanguage(), so the bundle never pays for
// languages the user never views.

import type { Root } from "hast";
import type { HighlighterGeneric, ThemeInput } from "shiki";

type Highlighter = HighlighterGeneric<string, string>;

const THEME_NAME = "hyprlayer";

// Token color scopes mapped to the app's status/brand/accent tokens. The shape
// matches codeView.ts (Lezer) one-for-one so highlight reads consistently
// whether you're looking at a chat block, a file in the viewer, or a diff.
// Cast to `ThemeInput` because Shiki accepts the VS Code Color Theme JSON
// shape (tokenColors) at runtime but its TS types model the raw TextMate
// format that requires a top-level `settings: []`.
const HYPRLAYER_THEME = {
  name: THEME_NAME,
  type: "dark",
  colors: {
    "editor.foreground": "var(--color-ink)",
    "editor.background": "transparent",
  },
  tokenColors: [
    {
      scope: ["comment", "punctuation.definition.comment"],
      settings: { foreground: "var(--color-ink-dim)", fontStyle: "italic" },
    },
    {
      scope: [
        "keyword",
        "keyword.control",
        "keyword.operator.expression",
        "keyword.operator.new",
        "storage.type",
        "storage.modifier",
      ],
      settings: { foreground: "var(--color-accent)" },
    },
    {
      scope: ["string", "string.quoted", "string.template", "string.regexp"],
      settings: { foreground: "var(--color-ok)" },
    },
    {
      scope: [
        "constant.numeric",
        "constant.language",
        "constant.character",
        "constant.other",
      ],
      settings: { foreground: "var(--color-warn)" },
    },
    {
      scope: ["entity.name.function", "support.function", "meta.function-call"],
      settings: { foreground: "var(--color-brand)" },
    },
    {
      scope: [
        "entity.name.type",
        "entity.name.class",
        "support.type",
        "support.class",
        "entity.name.namespace",
      ],
      settings: { foreground: "var(--color-brand-soft)" },
    },
    {
      scope: [
        "variable.other.property",
        "support.variable.property",
        "entity.other.attribute-name",
        "meta.attribute",
      ],
      settings: { foreground: "var(--color-brand-soft)" },
    },
    {
      scope: ["entity.name.tag", "meta.tag"],
      settings: { foreground: "var(--color-accent)" },
    },
    {
      scope: ["markup.heading"],
      settings: { foreground: "var(--color-ink)", fontStyle: "bold" },
    },
    { scope: ["markup.bold"], settings: { fontStyle: "bold" } },
    { scope: ["markup.italic"], settings: { fontStyle: "italic" } },
    {
      scope: ["invalid", "invalid.illegal"],
      settings: { foreground: "var(--color-bad)" },
    },
  ],
} as ThemeInput;

// Short aliases the model often emits as the fence info-string. Anything not
// listed passes through; Shiki recognizes most full language names natively.
const LANG_ALIASES: Record<string, string> = {
  js: "javascript",
  ts: "typescript",
  py: "python",
  rs: "rust",
  rb: "ruby",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  yml: "yaml",
  md: "markdown",
};

export function resolveLang(raw: string | undefined | null): string {
  if (!raw) return "text";
  const norm = raw.toLowerCase().trim();
  return LANG_ALIASES[norm] ?? norm;
}

let highlighterPromise: Promise<Highlighter> | null = null;

function getHighlighter(): Promise<Highlighter> {
  if (!highlighterPromise) {
    highlighterPromise = import("shiki").then((m) =>
      m.createHighlighter({ themes: [HYPRLAYER_THEME], langs: [] }),
    ) as Promise<Highlighter>;
  }
  return highlighterPromise;
}

// `loadedLangs` caches the post-load check so the steady-state highlight path
// is O(1) instead of an array scan into Shiki on every call. `loadingLangs`
// dedups concurrent loads so two parallel highlights for the same language
// don't kick off two dynamic-import chains.
const loadedLangs = new Set<string>();
const loadingLangs = new Map<string, Promise<void>>();

async function ensureLang(hl: Highlighter, lang: string): Promise<boolean> {
  if (loadedLangs.has(lang)) return true;
  if (hl.getLoadedLanguages().includes(lang)) {
    loadedLangs.add(lang);
    return true;
  }
  let pending = loadingLangs.get(lang);
  if (!pending) {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    pending = hl.loadLanguage(lang as any) as Promise<void>;
    loadingLangs.set(lang, pending);
  }
  try {
    await pending;
    loadedLangs.add(lang);
    return true;
  } catch {
    return false;
  } finally {
    loadingLangs.delete(lang);
  }
}

/** Highlight `code` as `rawLang` and return the resulting HAST tree (for
 *  rendering via hast-util-to-jsx-runtime). Returns null if the language is
 *  unknown or highlighting fails — the caller should render plain text. */
export async function highlightToHast(
  code: string,
  rawLang: string | undefined | null,
): Promise<{ hast: Root; lang: string } | null> {
  const lang = resolveLang(rawLang);
  if (lang === "text") return null;

  const hl = await getHighlighter();
  const ok = await ensureLang(hl, lang);
  if (!ok) return null;

  try {
    const hast = hl.codeToHast(code, { lang, theme: THEME_NAME });
    return { hast, lang };
  } catch {
    return null;
  }
}
