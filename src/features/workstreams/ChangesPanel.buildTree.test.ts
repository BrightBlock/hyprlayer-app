import { describe, expect, it } from "vitest";

import type { GitFileEntry } from "@/lib/bindings";
import { buildTree, type TreeNode } from "./ChangesPanel";

const added = (path: string): GitFileEntry => ({ path, change: "added" });

/** Flatten to `depth:name` rows the way TreeLevel renders them: a node with an
 *  entry and no children is a file leaf; anything else is a folder. */
function flatten(node: TreeNode, depth = -1): string[] {
  const rows: string[] = [];
  if (depth >= 0) rows.push(`${depth}:${node.name}${node.entry ? "" : "/"}`);
  const kids = [...node.children.values()].sort((a, b) => a.name.localeCompare(b.name));
  for (const child of kids) rows.push(...flatten(child, depth + 1));
  return rows;
}

describe("buildTree", () => {
  it("nests new files under their directories", () => {
    const tree = buildTree([added("crates/core/src/lib.rs"), added("crates/core/Cargo.toml")]);
    expect(flatten(tree)).toEqual([
      "0:crates/",
      "1:core/",
      "2:Cargo.toml",
      "2:src/",
      "3:lib.rs",
    ]);
  });

  it("never emits a nameless leaf for a collapsed trailing-slash dir entry", () => {
    // A collapsed untracked-dir entry (`crates/`) must not spawn a phantom
    // empty-named child, the bug that rendered a `+`/file-icon row with no label.
    const tree = buildTree([added("crates/")]);
    const names: string[] = [];
    const walk = (n: TreeNode) => {
      for (const c of n.children.values()) {
        names.push(c.name);
        walk(c);
      }
    };
    walk(tree);
    expect(names).toEqual(["crates"]);
    expect(names).not.toContain("");
  });

  it("ignores empty entry paths entirely", () => {
    const tree = buildTree([added(""), added("a.txt")]);
    expect([...tree.children.keys()]).toEqual(["a.txt"]);
  });
});
