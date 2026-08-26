
import type { Root } from "hast";
import type { HighlighterCore, LanguageRegistration, ThemeInput } from "shiki";

type Highlighter = HighlighterCore;

const THEME_NAME = "hyprlayer";

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

const LANG_ALIASES: Record<string, string> = {
  bash: "shellscript",
  batch: "bat",
  "c#": "csharp",
  "c++": "cpp",
  cjs: "javascript",
  clj: "clojure",
  cmd: "bat",
  console: "shellsession",
  cs: "csharp",
  cts: "typescript",
  dockerfile: "docker",
  erl: "erlang",
  "f#": "fsharp",
  fs: "fsharp",
  gql: "graphql",
  hbs: "handlebars",
  hs: "haskell",
  jl: "julia",
  js: "javascript",
  kt: "kotlin",
  kts: "kotlin",
  makefile: "make",
  md: "markdown",
  mjs: "javascript",
  mmd: "mermaid",
  mts: "typescript",
  objc: "objective-c",
  properties: "ini",
  protobuf: "proto",
  ps: "powershell",
  ps1: "powershell",
  py: "python",
  rb: "ruby",
  regex: "regexp",
  rs: "rust",
  sh: "shellscript",
  shell: "shellscript",
  tf: "terraform",
  tfvars: "terraform",
  ts: "typescript",
  yml: "yaml",
  zsh: "shellscript",
};

const GRAMMARS: Record<string, () => Promise<{ default: LanguageRegistration[] }>> = {
  astro: () => import("@shikijs/langs/astro"),
  awk: () => import("@shikijs/langs/awk"),
  bat: () => import("@shikijs/langs/bat"),
  c: () => import("@shikijs/langs/c"),
  clojure: () => import("@shikijs/langs/clojure"),
  cmake: () => import("@shikijs/langs/cmake"),
  cpp: () => import("@shikijs/langs/cpp"),
  csharp: () => import("@shikijs/langs/csharp"),
  css: () => import("@shikijs/langs/css"),
  csv: () => import("@shikijs/langs/csv"),
  dart: () => import("@shikijs/langs/dart"),
  diff: () => import("@shikijs/langs/diff"),
  docker: () => import("@shikijs/langs/docker"),
  elixir: () => import("@shikijs/langs/elixir"),
  erlang: () => import("@shikijs/langs/erlang"),
  fish: () => import("@shikijs/langs/fish"),
  fsharp: () => import("@shikijs/langs/fsharp"),
  "git-commit": () => import("@shikijs/langs/git-commit"),
  "git-rebase": () => import("@shikijs/langs/git-rebase"),
  go: () => import("@shikijs/langs/go"),
  graphql: () => import("@shikijs/langs/graphql"),
  groovy: () => import("@shikijs/langs/groovy"),
  handlebars: () => import("@shikijs/langs/handlebars"),
  haskell: () => import("@shikijs/langs/haskell"),
  hcl: () => import("@shikijs/langs/hcl"),
  html: () => import("@shikijs/langs/html"),
  http: () => import("@shikijs/langs/http"),
  ini: () => import("@shikijs/langs/ini"),
  java: () => import("@shikijs/langs/java"),
  javascript: () => import("@shikijs/langs/javascript"),
  json: () => import("@shikijs/langs/json"),
  json5: () => import("@shikijs/langs/json5"),
  jsonc: () => import("@shikijs/langs/jsonc"),
  jsx: () => import("@shikijs/langs/jsx"),
  julia: () => import("@shikijs/langs/julia"),
  kotlin: () => import("@shikijs/langs/kotlin"),
  latex: () => import("@shikijs/langs/latex"),
  less: () => import("@shikijs/langs/less"),
  lua: () => import("@shikijs/langs/lua"),
  make: () => import("@shikijs/langs/make"),
  markdown: () => import("@shikijs/langs/markdown"),
  matlab: () => import("@shikijs/langs/matlab"),
  mdx: () => import("@shikijs/langs/mdx"),
  mermaid: () => import("@shikijs/langs/mermaid"),
  nginx: () => import("@shikijs/langs/nginx"),
  nix: () => import("@shikijs/langs/nix"),
  "objective-c": () => import("@shikijs/langs/objective-c"),
  ocaml: () => import("@shikijs/langs/ocaml"),
  perl: () => import("@shikijs/langs/perl"),
  php: () => import("@shikijs/langs/php"),
  powershell: () => import("@shikijs/langs/powershell"),
  prisma: () => import("@shikijs/langs/prisma"),
  proto: () => import("@shikijs/langs/proto"),
  python: () => import("@shikijs/langs/python"),
  r: () => import("@shikijs/langs/r"),
  regexp: () => import("@shikijs/langs/regexp"),
  ruby: () => import("@shikijs/langs/ruby"),
  rust: () => import("@shikijs/langs/rust"),
  scala: () => import("@shikijs/langs/scala"),
  scss: () => import("@shikijs/langs/scss"),
  shellscript: () => import("@shikijs/langs/shellscript"),
  shellsession: () => import("@shikijs/langs/shellsession"),
  solidity: () => import("@shikijs/langs/solidity"),
  sql: () => import("@shikijs/langs/sql"),
  svelte: () => import("@shikijs/langs/svelte"),
  swift: () => import("@shikijs/langs/swift"),
  systemd: () => import("@shikijs/langs/systemd"),
  terraform: () => import("@shikijs/langs/terraform"),
  toml: () => import("@shikijs/langs/toml"),
  tsx: () => import("@shikijs/langs/tsx"),
  typescript: () => import("@shikijs/langs/typescript"),
  vue: () => import("@shikijs/langs/vue"),
  wasm: () => import("@shikijs/langs/wasm"),
  xml: () => import("@shikijs/langs/xml"),
  yaml: () => import("@shikijs/langs/yaml"),
  zig: () => import("@shikijs/langs/zig"),
};

export function resolveLang(raw: string | undefined | null): string {
  if (!raw) return "text";
  const norm = raw.toLowerCase().trim();
  return LANG_ALIASES[norm] ?? norm;
}

let highlighterPromise: Promise<Highlighter> | null = null;

function getHighlighter(): Promise<Highlighter> {
  if (!highlighterPromise) {
    highlighterPromise = Promise.all([
      import("shiki/core"),
      import("shiki/engine/oniguruma"),
    ]).then(([core, oniguruma]) =>
      core.createHighlighterCore({
        themes: [HYPRLAYER_THEME],
        langs: [],
        engine: oniguruma.createOnigurumaEngine(import("shiki/wasm")),
      }),
    );
  }
  return highlighterPromise;
}

const loadedLangs = new Set<string>();
const loadingLangs = new Map<string, Promise<void>>();

async function ensureLang(hl: Highlighter, lang: string): Promise<boolean> {
  if (loadedLangs.has(lang)) return true;
  if (hl.getLoadedLanguages().includes(lang)) {
    loadedLangs.add(lang);
    return true;
  }
  const grammar = GRAMMARS[lang];
  if (!grammar) return false;
  let pending = loadingLangs.get(lang);
  if (!pending) {
    pending = grammar().then((m) => hl.loadLanguage(m.default));
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
