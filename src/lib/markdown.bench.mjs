// Run by markdown.test.ts under --expose-gc with a large semi-space, so heapUsed deltas are allocations.
import { renderMarkdown } from "./markdown.ts";

function planLikeDoc(seed) {
  const section = (n) =>
    [
      `## Phase ${seed}.${n}: Wire the **runtime** into \`hyprlayer-core\``,
      "",
      `Paragraph ${n} with [a link](https://example.com/${n}), *emphasis*, \`inline code\` and a bare URL https://example.com/${seed}.`,
      "",
      "- [ ] first task with some description text",
      "- [x] second task that is already done",
      "  - nested item",
      "",
      "| Column | Value |",
      "| --- | --- |",
      `| row ${n} | ${n * 7} |`,
      "",
      "```rust",
      `fn phase_${n}() -> Result<(), CoreError> {`,
      '    let path = std::env::var("HOME").unwrap_or_default();',
      "    Ok(())",
      "}",
      "```",
      "",
      "> A quoted note that spans a line.",
      "",
    ].join("\n");
  let doc = "";
  for (let n = 0; doc.length < 160_000; n++) doc += section(n);
  return doc;
}

const mb = (n) => Math.round((n / 1048576) * 10) / 10;
const settle = () => {
  for (let i = 0; i < 4; i++) globalThis.gc();
  return process.memoryUsage().heapUsed;
};

renderMarkdown(planLikeDoc(0).slice(0, 4000));
const docA = planLikeDoc(1);
const docB = planLikeDoc(2);
const baseline = settle();

renderMarkdown(docA);
const allocated = process.memoryUsage().heapUsed - baseline;

renderMarkdown(docB);
const retained = settle() - baseline;

process.stdout.write(JSON.stringify({ docBytes: docA.length, allocatedMB: mb(allocated), retainedMB: mb(retained) }));
