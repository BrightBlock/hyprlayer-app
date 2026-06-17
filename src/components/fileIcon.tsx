// Theme-matched file-type icons: a small set of category glyphs (code / markup /
// style / data / image / doc) tinted with the app's `--color-*` theme tokens, so
// they recolor with the active theme instead of using fixed brand colors. Each
// category gets a distinct shape + hue, which reads as "different file types"
// while staying on-palette. Dependency-free.

import { BracesIcon, CodeIcon, DocIcon, HashIcon, ImageIcon } from "./icons";

type Category = "code" | "markup" | "style" | "data" | "image" | "doc";

// Category → glyph + theme-token text color (so it tracks the active theme).
const CATEGORY: Record<Category, { Glyph: typeof DocIcon; cls: string }> = {
  code: { Glyph: CodeIcon, cls: "text-brand" },
  markup: { Glyph: CodeIcon, cls: "text-accent" },
  style: { Glyph: HashIcon, cls: "text-brand-soft" },
  data: { Glyph: BracesIcon, cls: "text-warn" },
  image: { Glyph: ImageIcon, cls: "text-ok" },
  doc: { Glyph: DocIcon, cls: "text-ink-dim" },
};

const EXT_CATEGORY: Record<string, Category> = {
  // code / scripts
  ts: "code",
  js: "code",
  mjs: "code",
  cjs: "code",
  rs: "code",
  py: "code",
  go: "code",
  rb: "code",
  java: "code",
  c: "code",
  h: "code",
  cpp: "code",
  cc: "code",
  cxx: "code",
  hpp: "code",
  cs: "code",
  php: "code",
  swift: "code",
  kt: "code",
  kts: "code",
  sh: "code",
  bash: "code",
  zsh: "code",
  sql: "code",
  lua: "code",
  // markup
  tsx: "markup",
  jsx: "markup",
  html: "markup",
  htm: "markup",
  vue: "markup",
  svelte: "markup",
  xml: "markup",
  // styles
  css: "style",
  scss: "style",
  sass: "style",
  less: "style",
  // data / config
  json: "data",
  yaml: "data",
  yml: "data",
  toml: "data",
  ini: "data",
  env: "data",
  lock: "data",
  csv: "data",
  // images
  png: "image",
  jpg: "image",
  jpeg: "image",
  gif: "image",
  webp: "image",
  ico: "image",
  bmp: "image",
  avif: "image",
  svg: "image",
  // docs (explicit; everything unknown also lands here)
  md: "doc",
  mdx: "doc",
  txt: "doc",
  pdf: "doc",
};

function extOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** Theme-tinted category icon for a filename. */
export function FileTypeIcon({ name, size = 13 }: { name: string; size?: number }) {
  const category = EXT_CATEGORY[extOf(name)] ?? "doc";
  const { Glyph, cls } = CATEGORY[category];
  return (
    <span className={`shrink-0 ${cls}`}>
      <Glyph size={size} />
    </span>
  );
}
