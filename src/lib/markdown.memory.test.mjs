// .mjs so the app's browser-only typecheck never sees node: imports.
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { expect, it } from "vitest";

// Failure mode: a renderer change quietly brings back react-markdown-sized garbage
// (38MB per 150KB doc), or the render cache starts pinning more than the latest doc.
it("renders a 150KB doc within the allocation and retention budget", () => {
  const script = fileURLToPath(new URL("./markdown.bench.mjs", import.meta.url));
  const out = execFileSync(process.execPath, ["--expose-gc", "--max-semi-space-size=1024", script], { encoding: "utf8" });
  const { docBytes, allocatedMB, retainedMB } = JSON.parse(out);
  expect(docBytes).toBeGreaterThan(150_000);
  expect(allocatedMB).toBeLessThan(12);
  expect(retainedMB).toBeLessThan(3);
});
