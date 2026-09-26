import MarkdownIt from "markdown-it";
import { describe, expect, it, vi } from "vitest";

import { renderMarkdown } from "./markdown";

const html = (body: string) => renderMarkdown(body).html;

describe("renderMarkdown", () => {
  // Failure mode 1: raw HTML in a doc reaches the DOM (the preview sets innerHTML).
  it("escapes raw HTML instead of rendering it", () => {
    const out = html('<img src=x onerror="alert(1)">\n\ntext <script>x()</script>');
    expect(out).not.toContain("<img");
    expect(out).not.toContain("<script");
    expect(out).toContain("&lt;script&gt;");
  });

  // Failure mode 2: a javascript: link becomes clickable.
  it("drops javascript: link targets", () => {
    expect(html("[x](javascript:alert(1))")).not.toContain("href=\"javascript:");
  });

  // Failure mode 3: links navigate the app webview instead of opening externally.
  it("opens links in a new window without a referrer", () => {
    expect(html("[site](https://example.com)")).toContain(
      '<a href="https://example.com" target="_blank" rel="noreferrer">site</a>',
    );
  });

  // Failure mode 4: data-line is missing or off by one, breaking click-to-edit and the scrollspy.
  it("stamps each block with its 1-based source line", () => {
    const body = ["## Head", "", "para", "", "- item", "", "> quote", "", "---", "", "| a |", "| - |", "| 1 |"].join("\n");
    const out = html(body);
    expect(out).toContain('<h2 data-line="1">');
    expect(out).toContain('<p data-line="3">');
    expect(out).toContain('<ul data-line="5">');
    expect(out).toContain('<li data-line="5">');
    expect(out).toContain('<blockquote data-line="7">');
    expect(out).toContain('<hr data-line="9">');
    expect(out).toContain('<div class="md-table" data-line="11"><table');
  });

  // Failure mode 5: GFM task items render as a literal "[ ]" or lose their checked state.
  it("renders task list items as disabled checkboxes", () => {
    const out = html("- [ ] todo\n- [x] done");
    expect(out).not.toContain("[ ]");
    expect(out).not.toContain("[x]");
    expect(out).toContain('<input type="checkbox" disabled> todo');
    expect(out).toContain('<input type="checkbox" disabled checked> done');
  });

  // Failure mode 6: GFM strikethrough, tables and bare URLs stop rendering.
  it("keeps strikethrough, tables and autolinked URLs", () => {
    const out = html("~~gone~~ see https://example.com\n\n| h |\n| - |\n| c |");
    expect(out).toContain("<s>gone</s>");
    expect(out).toContain('href="https://example.com"');
    expect(out).toContain("<td>c</td>");
  });

  // Failure mode 7: a fence loses its language or code, or its code lands in the HTML string
  // (the preview mounts the highlighted block into the placeholder instead).
  it("turns fences into indexed placeholders carrying the exact code", () => {
    const { html: out, codeBlocks } = renderMarkdown("text\n\n```ts\nconst a = 1 < 2;\n```\n\n```\nplain\n```");
    expect(out).toContain('<div data-line="3" data-code-block="0"></div>');
    expect(out).toContain('<div data-line="7" data-code-block="1"></div>');
    expect(out).not.toContain("const a");
    expect(codeBlocks).toEqual([
      { code: "const a = 1 < 2;", lang: "ts" },
      { code: "plain", lang: undefined },
    ]);
  });

  // Failure mode 8: the same body is parsed twice when the headings and the preview both read it.
  it("parses an unchanged body once", () => {
    const parse = vi.spyOn(MarkdownIt.prototype, "parse");
    const body = `## once ${Math.random()}\n\ntext`;
    renderMarkdown(body);
    renderMarkdown(body);
    expect(parse).toHaveBeenCalledTimes(1);
    parse.mockRestore();
  });
});
