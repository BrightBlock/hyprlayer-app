// Exports recorded session logs from a profile's DB, read-only: `node export-real.mjs <hyprlayer.db> <outDir> [maxPerHarness]`.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";

const [db, out, cap = "40"] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const uri = `file:${db}?mode=ro`;
const sql = (q) => execFileSync("sqlite3", ["-readonly", "-json", uri, q], { maxBuffer: 1 << 30 }).toString();

const sessions = JSON.parse(
  sql(
    `select s.id, s.harness, count(*) n from sessions s join session_events e on e.session_id = s.id group by s.id order by n desc`,
  ) || "[]",
);
const taken = {};
for (const s of sessions) {
  taken[s.harness] = (taken[s.harness] ?? 0) + 1;
  if (taken[s.harness] > Number(cap) || s.n < 3) continue;
  const rows = JSON.parse(
    sql(`select seq, kind, payload, created_at from session_events where session_id = '${s.id.replace(/'/g, "")}' order by seq`) || "[]",
  );
  const lines = rows.map((r) => `{"seq":${r.seq},"kind":${JSON.stringify(r.kind)},"createdAt":${r.created_at},"event":${r.payload}}`);
  writeFileSync(path.join(out, `${s.harness}-${s.id.slice(0, 8)}.jsonl`), lines.join("\n") + "\n");
}
