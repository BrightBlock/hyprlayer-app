import { useMemo, useRef, type ReactNode } from "react";

/** One token per quoted string, `$VAR`/`%VAR%`, whitespace run, or word. */
const TOKEN_RE = /("[^"]*"|'[^']*'|\$\{?\w+\}?|%\w+%|\s+|[^\s]+)/g;
const LINE_PX = 22;
const ZWSP = "​";

/** Light shell highlighting: comments, the leading command, strings, env vars,
 *  and flags. Deliberately heuristic — enough colour to scan, not a real parser
 *  (matches the design mockup). */
function highlightLine(line: string): ReactNode {
  if (line.replace(/^\s+/, "").startsWith("#")) {
    return <span className="italic text-ink-dim/60">{line || ZWSP}</span>;
  }
  const parts = line.match(TOKEN_RE);
  if (!parts) return ZWSP;
  let sawCommand = false;
  return parts.map((p, i) => {
    if (/^\s+$/.test(p)) return <span key={i}>{p}</span>;
    let cls = "text-ink";
    if (/^["']/.test(p)) cls = "text-ship"; // string
    else if (/^[$%]/.test(p)) cls = "text-commit"; // env var
    else if (p.startsWith("-")) cls = "text-warn"; // flag
    else if (!sawCommand) {
      cls = "text-brand"; // the command itself
      sawCommand = true;
    }
    return (
      <span key={i} className={cls}>
        {p}
      </span>
    );
  });
}

type Props = {
  value: string;
  onChange: (next: string) => void;
  /** Min visible rows; the editor auto-grows beyond this with content. */
  minRows?: number;
  placeholder?: string;
  "aria-label"?: string;
};

/**
 * A small multi-line command editor: monospace `<textarea>` with a line-number
 * gutter and a synced syntax-highlight overlay. Auto-grows (no inner vertical
 * scroll) so the gutter, overlay, and textarea stay aligned without scroll math;
 * long lines scroll horizontally, with the overlay kept in sync. No external
 * dependencies — the app's CodeMirror is read-only/heavier and not wired for
 * editing here.
 */
export function ScriptEditor({
  value,
  onChange,
  minRows = 3,
  placeholder,
  "aria-label": ariaLabel,
}: Props) {
  const preRef = useRef<HTMLPreElement>(null);
  const lines = useMemo(() => value.split("\n"), [value]);
  const rows = Math.max(minRows, lines.length);

  return (
    <div className="flex overflow-hidden rounded-lg border border-edge bg-surface font-mono text-[12.5px] leading-[22px]">
      <div
        aria-hidden
        className="shrink-0 select-none border-r border-edge bg-white/[0.015] py-2 pl-2 pr-2.5 text-right text-ink-dim/50"
        style={{ minWidth: 36 }}
      >
        {Array.from({ length: rows }, (_, i) => (
          <div key={i} style={{ height: LINE_PX }}>
            {i + 1}
          </div>
        ))}
      </div>
      <div className="relative flex-1 overflow-hidden">
        <pre
          ref={preRef}
          aria-hidden
          className="pointer-events-none absolute inset-0 m-0 overflow-hidden whitespace-pre px-3 py-2 text-ink"
        >
          {lines.map((ln, i) => (
            <div key={i} style={{ height: LINE_PX }}>
              {highlightLine(ln)}
            </div>
          ))}
        </pre>
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onScroll={(e) => {
            const pre = preRef.current;
            if (pre) {
              pre.scrollLeft = e.currentTarget.scrollLeft;
              pre.scrollTop = e.currentTarget.scrollTop;
            }
          }}
          rows={rows}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          wrap="off"
          placeholder={placeholder}
          aria-label={ariaLabel}
          className="selectable relative z-10 block w-full resize-none overflow-auto whitespace-pre border-0 bg-transparent px-3 py-2 text-transparent caret-ink outline-none placeholder:text-ink-dim/50"
          style={{ lineHeight: `${LINE_PX}px` }}
        />
      </div>
    </div>
  );
}
