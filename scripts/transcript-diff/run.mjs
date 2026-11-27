// Replays recorded session logs through the TypeScript and Rust reducers and compares the resulting views.
// `node scripts/transcript-diff/run.mjs [--perf] [logDir...]`; defaults to every dir under target/transcript-diff/logs.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const work = path.join(root, "target/transcript-diff");
const here = path.join(root, "scripts/transcript-diff");
const args = process.argv.slice(2);
const perf = args.includes("--perf");
const dirs = args.filter((a) => !a.startsWith("--")).map((d) => path.resolve(d));
if (dirs.length === 0) {
  const logs = path.join(work, "logs");
  for (const d of readdirSync(logs)) dirs.push(path.join(logs, d));
}

const logs = dirs.flatMap((d) =>
  readdirSync(d)
    .filter((f) => f.endsWith(".jsonl"))
    .map((f) => ({ name: `${path.basename(d)}.${f.slice(0, -6)}`, file: path.join(d, f) })),
);
// e2e logs are named for their scenario; real ones start with their harness.
const group = (name) => name.split(".")[0] + (name.startsWith("real") ? `:${name.split(".")[1].split("-")[0]}` : "");

const bin = path.join(root, "target", perf ? "release" : "debug", "transcript-replay");
const cargo = ["build", "-p", "hyprlayer-client", "--bin", "transcript-replay", ...(perf ? ["--release"] : [])];
execFileSync("cargo", cargo, { cwd: root, stdio: "inherit", env: { ...process.env, CARGO_TARGET_DIR: path.join(root, "target") } });

if (perf) {
  const biggest = logs
    .map((l) => ({ ...l, size: statSync(l.file).size }))
    .sort((a, b) => b.size - a.size)[0];
  const run = (cmd, argv) => JSON.parse(execFileSync(cmd, argv, { cwd: here }).toString());
  const samples = (f) => Array.from({ length: 5 }, f).sort((a, b) => a.ms - b.ms)[2];
  const ts = samples(() =>
    run("node", ["--import", "./register.mjs", "--no-warnings", "ts-replay.mjs", "--perf", biggest.file, "3000"]),
  );
  const rs = samples(() => run(bin, ["--perf", biggest.file, "3000"]));
  console.log(`3,000-item replay of ${biggest.name} (median of 5, reducer loop only)`);
  console.log(`  ts   ${ts.events} events -> ${ts.items} items  ${ts.ms.toFixed(1)} ms`);
  console.log(`  rust ${rs.events} events -> ${rs.items} items  ${rs.ms.toFixed(1)} ms (release)`);
  process.exit(0);
}

const outTs = path.join(work, "out-ts");
const outRs = path.join(work, "out-rust");
for (const d of [outTs, outRs]) {
  rmSync(d, { recursive: true, force: true });
  mkdirSync(d, { recursive: true });
}
const pairs = logs.map((l) => `${l.name}=${l.file}`);
execFileSync("node", ["--import", "./register.mjs", "--no-warnings", "ts-replay.mjs", ...pairs, "--out", outTs], {
  cwd: here,
  stdio: "inherit",
  maxBuffer: 1 << 30,
});
execFileSync(bin, [...pairs, "--out", outRs], { stdio: "inherit" });

// A missing key and a null are the same to every reader of the view.
function normalize(value) {
  if (Array.isArray(value)) return value.map(normalize);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([, v]) => v !== null && v !== undefined)
        .map(([k, v]) => [k, normalize(v)]),
    );
  }
  return value;
}

function firstDiffs(a, b, at = "$", out = []) {
  if (out.length >= 5) return out;
  if (a === b) return out;
  const object = (v) => v && typeof v === "object";
  if (!object(a) || !object(b) || Array.isArray(a) !== Array.isArray(b)) {
    out.push(`${at}: ts=${JSON.stringify(a)?.slice(0, 160)} rust=${JSON.stringify(b)?.slice(0, 160)}`);
    return out;
  }
  for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) firstDiffs(a[key], b[key], `${at}.${key}`, out);
  return out;
}

const tally = {};
let failed = 0;
for (const { name } of logs) {
  const ts = normalize(JSON.parse(readFileSync(path.join(outTs, `${name}.json`), "utf8")));
  const rs = normalize(JSON.parse(readFileSync(path.join(outRs, `${name}.json`), "utf8")));
  const diffs = firstDiffs(ts, rs);
  const at = ts.fold.items.findIndex((h, i) => h !== rs.fold.items[i]);
  if (at !== -1) diffs.push(`fold item ${at}:\n    ts   ${ts.foldItems[at]?.slice(0, 400)}\n    rust ${rs.foldItems[at]?.slice(0, 400)}`);
  const g = (tally[group(name)] ??= { logs: 0, events: 0, differing: 0 });
  g.logs += 1;
  g.events += ts.events;
  if (diffs.length) {
    g.differing += 1;
    failed += 1;
    console.log(`DIFF ${name}\n  ${diffs.join("\n  ")}`);
  }
}
const real = Object.entries(tally).filter(([k]) => k.startsWith("real"));
const e2e = Object.entries(tally).filter(([k]) => !k.startsWith("real"));
const sum = (rows) => rows.reduce((n, [, v]) => n + v.logs, 0);
console.log(`\nlogs: ${logs.length} (e2e ${sum(e2e)}, real ${sum(real)})`);
for (const [k, v] of Object.entries(tally).filter(([k]) => k.startsWith("real"))) console.log(`  ${k}: ${v.logs} logs, ${v.events} events, ${v.differing} differing`);
console.log(`  e2e: ${sum(e2e)} logs, ${e2e.reduce((n, [, v]) => n + v.events, 0)} events, ${e2e.reduce((n, [, v]) => n + v.differing, 0)} differing`);
console.log(failed ? `${failed} log(s) differ` : "no differences");
process.exit(failed ? 1 : 0);
