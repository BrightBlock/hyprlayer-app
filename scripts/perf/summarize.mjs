#!/usr/bin/env node
// Prints median, range and run count per metric over target/perf/<client>/<n>/report.json.
// `--check` summarizes two hand-made reports and fails if the figures are wrong.

import { mkdtempSync, mkdirSync, readdirSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

export function median(xs) {
  const s = [...xs].sort((a, b) => a - b);
  const mid = s.length >> 1;
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

/** Spread as a share of the median, the number the 20% noise rule reads. */
export function summarize(reports) {
  const keys = new Set(reports.flatMap((r) => Object.keys(r.metrics ?? {})));
  const rows = {};
  for (const key of [...keys].sort()) {
    const xs = reports.map((r) => r.metrics?.[key]).filter((v) => typeof v === "number");
    if (!xs.length) continue;
    const med = median(xs);
    const [min, max] = [Math.min(...xs), Math.max(...xs)];
    rows[key] = { median: med, min, max, runs: xs.length, spreadPct: med ? ((max - min) / med) * 100 : 0 };
  }
  return rows;
}

export function load(dir) {
  return readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .sort((a, b) => Number(a.name) - Number(b.name))
    .map((e) => JSON.parse(readFileSync(join(dir, e.name, "report.json"), "utf8")));
}

const fmt = (n) => (Number.isInteger(n) ? String(n) : n.toFixed(2));

function print(client, reports) {
  const errors = reports.reduce((n, r) => n + (r.errors?.length ?? 0), 0);
  console.log(`${client}: ${reports.length} runs, ${errors} errors`);
  for (const [key, r] of Object.entries(summarize(reports))) {
    const flag = r.spreadPct > 20 ? "  SPREAD>20%" : "";
    console.log(`  ${key}: ${fmt(r.median)} (${fmt(r.min)} to ${fmt(r.max)}), n=${r.runs}, spread ${r.spreadPct.toFixed(1)}%${flag}`);
  }
}

function check() {
  const dir = mkdtempSync(join(tmpdir(), "perf-summarize-"));
  try {
    const data = [{ a: 10, b: 4 }, { a: 20, b: 4 }];
    data.forEach((metrics, i) => {
      mkdirSync(join(dir, "fake", String(i + 1)), { recursive: true });
      writeFileSync(join(dir, "fake", String(i + 1), "report.json"), JSON.stringify({ metrics, errors: [] }));
    });
    const rows = summarize(load(join(dir, "fake")));
    const ok =
      rows.a.median === 15 && rows.a.min === 10 && rows.a.max === 20 && rows.a.runs === 2 &&
      rows.b.median === 4 && rows.b.spreadPct === 0 &&
      median([3, 1, 2]) === 2;
    if (!ok) {
      console.error("summarize check failed", JSON.stringify(rows));
      process.exit(1);
    }
    console.log("summarize check ok");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args.includes("--check")) check();
  else {
    const base = args[0] ?? join(root, "target", "perf");
    for (const e of readdirSync(base, { withFileTypes: true }).filter((e) => e.isDirectory())) {
      print(e.name, load(join(base, e.name)));
    }
  }
}
