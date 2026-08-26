
import { describe, expect, it } from "vitest";
import { highlightToHast, resolveLang } from "./shikiHighlight";

describe("shikiHighlight", () => {
  it("highlights canonical + aliased + embedded-dep languages", async () => {
    for (const [fence, expected] of [
      ["typescript", "typescript"],
      ["ts", "typescript"],
      ["bash", "shellscript"],
      ["c++", "cpp"],
      ["objc", "objective-c"],
      ["html", "html"],
      ["vue", "vue"],
      ["yml", "yaml"],
      ["tf", "terraform"],
    ] as const) {
      expect(resolveLang(fence)).toBe(expected);
      const r = await highlightToHast("const x = 1\n", fence);
      expect(r, `fence ${fence}`).not.toBeNull();
      expect(r!.lang).toBe(expected);
      expect(JSON.stringify(r!.hast)).toContain("var(--color-");
    }
  }, 60_000);

  it("covers the fence languages in real documents", async () => {
    const inUse = [
      "bash", "cmd", "console", "css", "csv", "diff", "go", "html",
      "javascript", "js", "json", "jsonc", "jsx", "latex", "markdown", "md",
      "mdx", "mermaid", "mjs", "powershell", "python", "ruby", "rust", "sh",
      "shell", "sql", "toml", "ts", "tsx", "typescript", "yaml",
    ];
    for (const fence of inUse) {
      expect(await highlightToHast("x = 1\n", fence), `fence ${fence}`).not.toBeNull();
    }
  }, 60_000);

  it("falls back to plain text for a dropped grammar", async () => {
    expect(await highlightToHast("(defun f ())", "emacs-lisp")).toBeNull();
    expect(await highlightToHast("x", "wolfram")).toBeNull();
  }, 30_000);
});
