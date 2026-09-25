
import { Icon, type IconName, type IconSize } from "./Icon";

type Category = "code" | "markup" | "style" | "data" | "image" | "doc";

const CATEGORY: Record<Category, { name: IconName; cls: string }> = {
  code: { name: "code", cls: "text-brand" },
  markup: { name: "code", cls: "text-accent" },
  style: { name: "hash", cls: "text-brand-soft" },
  data: { name: "braces", cls: "text-warn-ink" },
  image: { name: "image", cls: "text-ok" },
  doc: { name: "doc", cls: "text-ink-dim" },
};

const EXT_CATEGORY: Record<string, Category> = {
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
  tsx: "markup",
  jsx: "markup",
  html: "markup",
  htm: "markup",
  vue: "markup",
  svelte: "markup",
  xml: "markup",
  css: "style",
  scss: "style",
  sass: "style",
  less: "style",
  json: "data",
  yaml: "data",
  yml: "data",
  toml: "data",
  ini: "data",
  env: "data",
  lock: "data",
  csv: "data",
  png: "image",
  jpg: "image",
  jpeg: "image",
  gif: "image",
  webp: "image",
  ico: "image",
  bmp: "image",
  avif: "image",
  svg: "image",
  md: "doc",
  mdx: "doc",
  txt: "doc",
  pdf: "doc",
};

function extOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

export function FileTypeIcon({ name, size = "xs" }: { name: string; size?: IconSize }) {
  const category = EXT_CATEGORY[extOf(name)] ?? "doc";
  const glyph = CATEGORY[category];
  return (
    <span className={`shrink-0 ${glyph.cls}`}>
      <Icon name={glyph.name} size={size} />
    </span>
  );
}
