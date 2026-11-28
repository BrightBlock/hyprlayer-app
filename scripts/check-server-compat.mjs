#!/usr/bin/env node
// Fails when the app calls an RPC that the oldest server it accepts
// (MIN_SERVER_VERSION) lacks or has with a different signature (ADR-0009).
// A call on the line after `// server-compat: gated on <feature>` is made only
// to a server whose hello lists that feature, so it is checked only when the
// oldest server's hyprlayer-proto names the feature.
// Usage: check-server-compat.mjs [--root <repo>]

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootArg = process.argv.indexOf("--root");
const root = rootArg > 0 ? process.argv[rootArg + 1] : join(dirname(fileURLToPath(import.meta.url)), "..");

const RPC = "crates/hyprlayer-server/src/rpc.rs";
const PROTO = "crates/hyprlayer-proto/src/lib.rs";
const SERVER_TOML = "crates/hyprlayer-server/Cargo.toml";
const MIN_FILE = "crates/hyprlayer-client/src/server_version.rs";
const APP_ROOTS = [
  "src-tauri/src",
  "crates/hyprlayer-client/src",
  "crates/hyprlayer-desktop/src",
  "crates/hyprlayer-ui/src",
];

const git = (...args) =>
  execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
const tagExists = (tag) => {
  try {
    git("rev-parse", "-q", "--verify", `refs/tags/${tag}`);
    return true;
  } catch {
    return false;
  }
};
const read = (path) => readFileSync(join(root, path), "utf8");

const report = (lines) => console.error(lines.map((l, i) => (i ? `  ${l}` : `✗ ${l}`)).join("\n"));
function die(lines) {
  report(lines);
  process.exit(1);
}

const versionKey = (v) => v.replace(/\+.*$/, "").split(".").map(Number);
function cmpVersion(a, b) {
  const [x, y] = [versionKey(a), versionKey(b)];
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
  return 0;
}

const stripLineComments = (src) => src.replace(/^[ \t]*\/\/.*$/gm, "");
// Drop whitespace except between two word characters (`dyn Foo`, `&'a mut T`).
const normalize = (s) =>
  s
    .replace(/\s+/g, " ")
    .replace(/ (?=\W)|(?<=\W) /g, "")
    .trim();

// method -> `Variant(args) -> Ret`: the variant is the wire tag and arg names are its body fields.
function parseTable(src, where) {
  const start = src.search(/hyprlayer_proto::commands!\s*\{/);
  if (start < 0) die([`no hyprlayer_proto::commands! table in ${where}`]);
  // No string literals in the table, so any `//` starts a comment.
  const body = src.slice(src.indexOf("{", start) + 1).replace(/\/\/.*$/gm, "");
  const entries = [];
  let depth = 0;
  let cur = "";
  // `<>` isn't tracked: `->` would unbalance it, and only `[T; N]` puts a `;` inside a type.
  for (const ch of body) {
    if (depth === 0 && ch === "}") break;
    if ("([{".includes(ch)) depth++;
    if (")]}".includes(ch)) depth--;
    if (depth === 0 && ch === ";") {
      entries.push(cur.trim());
      cur = "";
    } else cur += ch;
  }
  const table = new Map();
  for (const entry of entries) {
    if (/^client\s*=/.test(entry)) continue;
    const m = entry.match(/^(\w+)\s*:\s*(\w+)\s*\(([\s\S]*)\)\s*->\s*([\s\S]+)$/);
    if (!m) die([`unparseable command table entry in ${where}:`, entry]);
    const [, variant, method, args, ret] = m;
    table.set(method, { variant, sig: `${variant}(${normalize(args).replace(/,$/, "")}) -> ${normalize(ret)}` });
  }
  return table;
}

function tableAt(tag) {
  let src;
  try {
    src = git("show", `${tag}:${RPC}`);
  } catch {
    die([`${tag} has no ${RPC} to compare against.`]);
  }
  return parseTable(src, `${tag}:${RPC}`);
}

function rustFiles(dir) {
  return readdirSync(join(root, dir), { withFileTypes: true })
    .sort((a, b) => a.name.localeCompare(b.name))
    .flatMap((e) =>
    e.isDirectory() ? rustFiles(join(dir, e.name)) : e.name.endsWith(".rs") ? [join(dir, e.name)] : [],
  );
}

const GATE = /^\s*\/\/\s*server-compat: gated on (\w+)\s*$/;

// Any `.method(` naming a table command counts, wherever the client came from;
// a same-named method on another type fails closed.
function appCalls(table) {
  const byVariant = new Map([...table].map(([method, { variant }]) => [variant, method]));
  const calls = new Map();
  const gated = new Map();
  for (const file of APP_ROOTS.flatMap(rustFiles)) {
    const raw = read(file);
    const lines = raw.split("\n");
    // Stripping keeps every newline, so a match's line is the same in both.
    const src = stripLineComments(raw);
    const note = (method, index) => {
      const line = src.slice(0, index).split("\n").length - 1;
      const feature = lines[line - 1]?.match(GATE)?.[1];
      if (!feature) calls.has(method) || calls.set(method, file);
      else gated.has(method) || gated.set(method, { file, feature });
    };
    for (const m of src.matchAll(/\.\s*(\w+)\s*(?:::<[^>]*>)?\s*\(/g)) if (table.has(m[1])) note(m[1], m.index);
    for (const m of src.matchAll(/\bRequest::(\w+)/g)) if (byVariant.has(m[1])) note(byVariant.get(m[1]), m.index);
  }
  for (const method of calls.keys()) gated.delete(method);
  return { calls, gated };
}

const names = (src, feature) => src.includes(`"${feature}"`);
function protoAt(tag) {
  try {
    return git("show", `${tag}:${PROTO}`);
  } catch {
    return "";
  }
}

const serverVersion = read(SERVER_TOML).match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!serverVersion) die([`no version line in ${SERVER_TOML}`]);
const minVersion = read(MIN_FILE).match(/MIN_SERVER_VERSION\s*:\s*&str\s*=\s*"([^"]+)"/)?.[1];
if (!minVersion) die([`no MIN_SERVER_VERSION in ${MIN_FILE}`]);

if (cmpVersion(minVersion, serverVersion) > 0)
  die([
    `MIN_SERVER_VERSION ${minVersion} is above the server crate's version ${serverVersion}.`,
    `Lower it in ${MIN_FILE}, or bump the version in ${SERVER_TOML}.`,
  ]);

const head = parseTable(read(RPC), RPC);
const failures = [];
let skipped = 0;

const serverTag = `server-v${serverVersion}`;
let needsBump = false;
if (tagExists(serverTag)) {
  const released = tableAt(serverTag);
  const drift = [...new Set([...head.keys(), ...released.keys()])].filter(
    (m) => head.get(m)?.sig !== released.get(m)?.sig,
  );
  needsBump = drift.length > 0;
  if (needsBump)
    failures.push([
      `the command table changed since ${serverTag}, but crates/hyprlayer-server is still ${serverVersion}:`,
      ...drift.flatMap((m) => [`${m}:`, `  ${serverTag}: ${released.get(m)?.sig ?? "(absent)"}`, `  now: ${head.get(m)?.sig ?? "(removed)"}`]),
      `Bump the version in ${SERVER_TOML}.`,
    ]);
}

// MIN equal to the server version is this tree, possibly not tagged until release.sh publishes it.
const minTag = `server-v${minVersion}`;
if (cmpVersion(minVersion, serverVersion) < 0) {
  if (!tagExists(minTag))
    die([
      `tag ${minTag} (MIN_SERVER_VERSION) is not in this clone, so its command table can't be compared.`,
      "Run `git fetch --tags` and re-run.",
    ]);
  const min = tableAt(minTag);
  const { calls, gated } = appCalls(head);
  const [headProto, minProto] = gated.size ? [existsSync(join(root, PROTO)) ? read(PROTO) : "", protoAt(minTag)] : ["", ""];
  const unknown = [];
  for (const [method, { file, feature }] of gated) {
    if (!names(headProto, feature)) unknown.push(`${method} (called in ${file}): gated on ${feature}, which ${PROTO} doesn't name`);
    else if (names(minProto, feature)) calls.set(method, file);
    else skipped++;
  }
  const broken = [...calls]
    .sort(([a], [b]) => a.localeCompare(b))
    .flatMap(([method, file]) => {
      const was = min.get(method);
      if (!was) return [`${method} (called in ${file}): not in ${minTag}`];
      if (was.sig !== head.get(method).sig)
        return [`${method} (called in ${file}): signature differs`, `  ${minTag}: ${was.sig}`, `  now: ${head.get(method).sig}`];
      return [];
    });
  if (broken.length)
    failures.push([
      `the app calls commands that server ${minVersion} (MIN_SERVER_VERSION) lacks or has with another signature:`,
      ...broken,
      needsBump
        ? `Bump the version in ${SERVER_TOML}, then raise MIN_SERVER_VERSION in ${MIN_FILE} to it.`
        : `Raise MIN_SERVER_VERSION in ${MIN_FILE} to ${serverVersion}, the crates/hyprlayer-server version.`,
    ]);
  if (unknown.length)
    failures.push([
      "the app gates calls on hello features the server doesn't have:",
      ...unknown,
      `Name the feature as it is in ${PROTO}.`,
    ]);
}

if (failures.length) {
  failures.forEach(report);
  process.exit(1);
}
const gatedNote = skipped ? `; ${skipped} gated on hello features it lacks` : "";
console.log(`✓ server compat: app RPCs match server ${minVersion} (MIN_SERVER_VERSION); server ${serverVersion}${gatedNote}`);
