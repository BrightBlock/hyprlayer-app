#!/usr/bin/env node
// Folds the client's report, the warm relaunch and the harness's samples into one report.json.
// Usage: merge.mjs <run dir>

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2];
const json = (p) => (existsSync(p) ? JSON.parse(readFileSync(p, "utf8")) : null);
const client = json(join(dir, "client.json")) ?? { metrics: {}, errors: ["no client report"], proof: {} };
const warm = json(join(dir, "warm.json"));
const sample = (name) => json(join(dir, "samples", `${name}.json`));
const mb = (kb) => Math.round((kb / 1024) * 10) / 10;

const metrics = { ...client.metrics };
if ("m1.first_interactive_ms" in metrics) {
  metrics["m1.cold_ms"] = metrics["m1.first_interactive_ms"];
  delete metrics["m1.first_interactive_ms"];
}
if (warm?.metrics?.["m1.first_interactive_ms"] != null) metrics["m1.warm_ms"] = warm.metrics["m1.first_interactive_ms"];

const settled = sample("settled");
const soaked = sample("soaked");
if (settled?.pssKb != null) {
  metrics["m2.settled_mb"] = mb(settled.pssKb);
  metrics["m2.gpu_settled_mb"] = mb(settled.gpuKb ?? 0);
}
if (soaked?.pssKb != null) {
  metrics["m2.soaked_mb"] = mb(soaked.pssKb);
  metrics["m2.gpu_soaked_mb"] = mb(soaked.gpuKb ?? 0);
  if (settled?.pssKb) metrics["m2.growth_pct"] = Math.round(((soaked.pssKb - settled.pssKb) / settled.pssKb) * 1000) / 10;
}
for (const name of ["seq", "cat"]) {
  const a = sample(`${name}_begin`);
  const b = sample(`${name}_end`);
  if (a?.cpuMs != null && b?.cpuMs != null) metrics[`m4.${name}.cpu_ms`] = Math.round(b.cpuMs - a.cpuMs);
}

// Per-thread CPU over each terminal run: what the limiter claim reads.
const limiters = {};
for (const name of ["seq", "cat"]) {
  const a = sample(`${name}_begin`)?.threads ?? [];
  const b = sample(`${name}_end`)?.threads ?? [];
  const before = new Map(a.map((t) => [`${t.pid}:${t.tid}`, t.cpuMs]));
  limiters[name] = b
    .map((t) => ({ pid: t.pid, name: t.name, cpuMs: Math.round(t.cpuMs - (before.get(`${t.pid}:${t.tid}`) ?? 0)) }))
    .filter((t) => t.cpuMs > 0)
    .sort((x, y) => y.cpuMs - x.cpuMs)
    .slice(0, 6);
  const procs = {};
  for (const p of sample(`${name}_end`)?.procs ?? []) {
    const prev = (sample(`${name}_begin`)?.procs ?? []).find((q) => q.pid === p.pid);
    procs[`${p.pid}:${p.comm}`] = Math.round(p.cpuMs - (prev?.cpuMs ?? 0));
  }
  limiters[`${name}_procs`] = procs;
}

const errors = [...(client.errors ?? [])];
if (warm?.errors?.length) errors.push(...warm.errors.map((e) => `warm: ${e}`));
writeFileSync(
  join(dir, "report.json"),
  JSON.stringify({ client: client.client ?? "tauri", scenario: client.scenario, errors, metrics, proof: client.proof, limiters, processes: settled?.procs ?? [] }, null, 2),
);
console.log(`${dir}: ${Object.keys(metrics).length} metrics, ${errors.length} errors`);
