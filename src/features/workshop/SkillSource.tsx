import { useMemo } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { RangeSetBuilder, type Text } from "@codemirror/state";
import { Decoration, EditorView } from "@codemirror/view";

import { ErrorNote, Spinner } from "@/components/ui";
import type { SkillFile } from "@/lib/bindings";
import { readOnlyExtensions, useLangForPath } from "@/lib/codeView";
import { useSkillFileText } from "@/lib/queries";

import { skillTokenLine } from "./skills";

const BASIC_SETUP = {
  foldGutter: false,
  lineNumbers: false,
  syntaxHighlighting: false,
  highlightActiveLine: false,
  highlightActiveLineGutter: false,
} as const;

const frontmatterLine = Decoration.line({ class: "cm-skill-frontmatter" });

// Markdown reads the block's last key plus its closing `---` as a setext heading.
function frontmatterLines(doc: Text) {
  const builder = new RangeSetBuilder<Decoration>();
  if (doc.lines > 1 && doc.line(1).text === "---") {
    for (let n = 2; n <= doc.lines; n++) {
      if (doc.line(n).text !== "---") continue;
      for (let i = 1; i <= n; i++) builder.add(doc.line(i).from, doc.line(i).from, frontmatterLine);
      break;
    }
  }
  return builder.finish();
}

const dimFrontmatter = [
  EditorView.decorations.compute(["doc"], (state) => frontmatterLines(state.doc)),
  EditorView.theme({
    ".cm-skill-frontmatter, .cm-skill-frontmatter *": {
      color: "var(--color-ink-dim) !important",
      fontWeight: "normal !important",
    },
  }),
];

export function SkillSource({ skill, projectPath }: { skill: SkillFile; projectPath: string | null }) {
  const text = useSkillFileText(projectPath, skill.path);
  const lang = useLangForPath(skill.path);
  const extensions = useMemo(() => [...readOnlyExtensions(lang, { gutter: false }), dimFrontmatter], [lang]);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-card border border-edge bg-panel">
      <div className="flex h-9 shrink-0 items-center gap-3 border-b border-edge/60 px-4">
        <span className="text-meta font-semibold text-ink">SKILL.md</span>
        <span className="text-meta text-ink-dim">body · markdown</span>
        <span className="flex-1" />
        <span className="text-meta text-ink-dim">{skillTokenLine(skill)}</span>
      </div>
      {text.isLoading ? (
        <div className="flex h-24 items-center justify-center">
          <Spinner />
        </div>
      ) : text.isError ? (
        <div className="p-4">
          <ErrorNote error={text.error} />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
          <CodeMirror
            value={text.data ?? ""}
            theme="none"
            editable={false}
            extensions={extensions}
            basicSetup={BASIC_SETUP}
          />
        </div>
      )}
    </div>
  );
}
