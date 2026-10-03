#!/usr/bin/env node
// Pulls the three v4 illustrations out of the canvas exports into src/assets/art/ as stroke-only SVGs.
// Usage: node scripts/extract-blank-art.mjs <dir holding BlankFirstRun.dc.html …>
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const SOURCES = {
  "first-run": ["BlankFirstRun", "Onboarding1", "Onboarding4"],
  session: ["BlankSession", "Onboarding2"],
  library: ["BlankLibrary", "Onboarding3"],
};
const ART = /<svg viewBox="0 0 1440 940"[^>]*>([\s\S]*?)<\/svg>/;

function strokes(file) {
  const match = readFileSync(file, "utf8").match(ART);
  if (!match) throw new Error(`no 1440x940 art in ${file}`);
  return match[1]
    .replace(/<defs>[\s\S]*?<\/defs>/, "")
    .replace(/<rect width="1440" height="940"[^>]*>(<\/rect>)?/, "");
}

const [dir] = process.argv.slice(2);
if (!dir) throw new Error("usage: extract-blank-art.mjs <design dir>");
const out = new URL("../src/assets/art/", import.meta.url);
mkdirSync(out, { recursive: true });
for (const [name, files] of Object.entries(SOURCES)) {
  const bodies = files.map((f) => strokes(join(dir, `${f}.dc.html`)));
  if (bodies.some((b) => b !== bodies[0])) throw new Error(`${name}: ${files.join(", ")} disagree`);
  // Standalone files need the namespace or WebKit won't decode them as images.
  writeFileSync(new URL(`${name}.svg`, out), `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1440 940">${bodies[0]}</svg>\n`);
}
