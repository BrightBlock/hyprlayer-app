/**
 * Shared inline SVG icons. Stroke-based and sized via props, so every surface
 * renders the same glyph at its own scale instead of redrawing it.
 */

type IconProps = {
  size?: number;
  strokeWidth?: number;
  className?: string;
};

export function PlusIcon({ size = 16, strokeWidth = 2 }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

export function XIcon({ size = 14, strokeWidth = 2 }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="m7 7 10 10M17 7 7 17" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

export function WarningTriangleIcon({ size = 14, strokeWidth = 2, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <path
        d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h16.9a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinejoin="round"
      />
      <path d="M12 9v4" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
      <path d="M12 17h.01" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

export function GitPullRequestIcon({ size = 16, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <circle cx="6" cy="6" r="2.5" stroke="currentColor" strokeWidth={strokeWidth} />
      <circle cx="18" cy="18" r="2.5" stroke="currentColor" strokeWidth={strokeWidth} />
      <path d="M6 8.5V18a3 3 0 0 0 3 3h1" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
      <path d="M18 15.5V10a4 4 0 0 0-4-4h-2" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
      <path d="m14 3-3 3 3 3" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function GitCommitIcon({ size = 16, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <circle cx="12" cy="12" r="3.5" stroke="currentColor" strokeWidth={strokeWidth} />
      <path d="M3 12h5M16 12h5" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

export function SplitIcon({ size = 16, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <rect x="3" y="4.5" width="18" height="15" rx="2.5" stroke="currentColor" strokeWidth={strokeWidth} />
      <path d="M12 5v14" stroke="currentColor" strokeWidth={strokeWidth} strokeDasharray="2 2" />
    </svg>
  );
}

export function ImageIcon({ size = 15, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <rect x="3" y="4.5" width="18" height="15" rx="2.5" />
      <circle cx="8.5" cy="9.5" r="1.6" />
      <path d="m4 17 4.5-4.5a1.5 1.5 0 0 1 2 0L17 19" />
    </svg>
  );
}

export function BookIcon({ size = 15, strokeWidth = 1.8 }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path
        d="M4 19.5V5a2 2 0 0 1 2-2h12v18H6a2 2 0 0 1-2-1.5Z"
        stroke="currentColor"
        strokeWidth={strokeWidth}
      />
      <path d="M8 7h6M8 11h7" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

export function ChevronRightIcon({ size = 16, strokeWidth = 2, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path d="m9 6 6 6-6 6" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function ChevronLeftIcon({ size = 16, strokeWidth = 2, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path d="m15 6-6 6 6 6" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function ChevronDownIcon({ size = 14, strokeWidth = 2, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path d="m6 9 6 6 6-6" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/** Four-point sparkle — the model selector glyph. */
export function SparkleIcon({ size = 14, strokeWidth = 1.6, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path
        d="M12 3c.4 3.6 1.4 4.6 5 5-3.6.4-4.6 1.4-5 5-.4-3.6-1.4-4.6-5-5 3.6-.4 4.6-1.4 5-5Z"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Ascending signal bars — the reasoning-effort glyph. */
export function EffortIcon({ size = 14, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="currentColor" className={className}>
      <rect x="3" y="14" width="3.5" height="7" rx="1" />
      <rect x="8.5" y="10" width="3.5" height="11" rx="1" />
      <rect x="14" y="6" width="3.5" height="15" rx="1" />
      <rect x="19.5" y="3" width="1.5" height="18" rx="0.75" opacity="0.55" />
    </svg>
  );
}

export function DocIcon({ size = 12, strokeWidth = 2 }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M6 3h8l4 4v14H6V3Z" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
      <path d="M14 3v4h4" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
    </svg>
  );
}

/** Pencil — a file edit. */
export function PencilIcon({ size = 12, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <path d="M4 20h4L18.5 9.5a2 2 0 0 0-2.83-2.83L5 17.5V20Z" />
      <path d="m14 8 2.5 2.5" />
    </svg>
  );
}

/** Magnifying glass — the document's preview/read mode. */
export function SearchIcon({ size = 14, strokeWidth = 1.9, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </svg>
  );
}

/** `< >` — source/code files. */
export function CodeIcon({ size = 13, strokeWidth = 1.9, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <path
        d="m9 8-4 4 4 4M15 8l4 4-4 4"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** A nested-hexagon mark standing in for the OpenAI/Codex harness. */
export function OpenAiIcon({ size = 13, strokeWidth = 1.6, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <path d="M12 2 20 7 V17 L12 22 4 17 V7 Z" />
      <path d="M12 7 16 9.5 V14.5 L12 17 8 14.5 V9.5 Z" />
    </svg>
  );
}

/** A terminal window: `>_` prompt — terminal emulators. */
export function TerminalIcon({ size = 13, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <rect x="3" y="4" width="18" height="16" rx="2" stroke="currentColor" strokeWidth={strokeWidth} />
      <path
        d="m7 9 3 3-3 3M13 15h4"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Checklist — a ticked box over ruled lines. */
export function ChecklistIcon({ size = 14, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <path
        d="m3 6 1.4 1.4L7.2 4.4"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path d="M11 6h10" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
      <path d="M4 12h17" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
      <path d="M4 18h17" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

/** Two stacked sheets — copy to clipboard. */
export function ClipboardIcon({ size = 13, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <rect x="9" y="9" width="11" height="11" rx="2" stroke="currentColor" strokeWidth={strokeWidth} />
      <path
        d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Curved back-arrow — the "rewind to here" prompt toolbar action. */
export function RewindIcon({ size = 13, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <path
        d="M9 14 4 9l5-5"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M4 9h9a6 6 0 0 1 6 6v0a6 6 0 0 1-6 6h-3"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** `{ }` — data/config files (json, yaml, toml…). */
export function BracesIcon({ size = 13, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <path
        d="M8 4a2 2 0 0 0-2 2v3a2 2 0 0 1-2 2 2 2 0 0 1 2 2v3a2 2 0 0 0 2 2M16 4a2 2 0 0 1 2 2v3a2 2 0 0 0 2 2 2 2 0 0 0-2 2v3a2 2 0 0 1-2 2"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** `#` — stylesheet files (css, scss…). */
export function HashIcon({ size = 13, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <path
        d="M9 4 7 20M17 4l-2 16M5 9h14M4 15h14"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function FolderIcon({ size = 13, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <path
        d="M3 6.5A1.5 1.5 0 0 1 4.5 5h4l2 2.5h7A1.5 1.5 0 0 1 19 9v8.5A1.5 1.5 0 0 1 17.5 19h-13A1.5 1.5 0 0 1 3 17.5v-11Z"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** A "+" stacked over a "−": the add/remove glyph that marks a diff tab apart
 *  from a plain file view. */
export function DiffIcon({ size = 13, strokeWidth = 2, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
    >
      <path
        d="M12 4v7M8.5 7.5h7M8.5 17.5h7"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
      />
    </svg>
  );
}

/** Upward arrow — the "send turn" affordance. */
export function SendIcon({ size = 16, strokeWidth = 2 }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M12 19V5M6 11l6-6 6 6" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/** Down-arrow into a tray — download / install an update. */
export function DownloadIcon({ size = 14, strokeWidth = 1.9, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <path d="M12 3v12" />
      <path d="m7 11 5 5 5-5" />
      <path d="M5 21h14" />
    </svg>
  );
}

/** Filled square — interrupt / stop the running turn. */
export function EraserIcon({ size = 14, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path
        d="m7 21-4.3-4.3a1.7 1.7 0 0 1 0-2.4l9.6-9.6a1.7 1.7 0 0 1 2.4 0l5.6 5.6a1.7 1.7 0 0 1 0 2.4L13 21"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path d="M22 21H7" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
      <path d="m5 11 9 9" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

export function StopIcon({ size = 14 }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <rect x="6" y="6" width="12" height="12" rx="2" />
    </svg>
  );
}

/** Circular arrow — restart / relaunch an agent. */
export function RestartIcon({ size = 13, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <path d="M21 12a9 9 0 1 1-2.64-6.36" />
      <path d="M21 3v5h-5" />
    </svg>
  );
}

/** Four panes — the Multiplexer mode mark (Settings hero + title-bar chip). */
export function MultiplexerIcon({ size = 14, strokeWidth = 1.6, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      className={className}
    >
      <rect x="3" y="3" width="7.5" height="7.5" rx="1.5" />
      <rect x="13.5" y="3" width="7.5" height="7.5" rx="1.5" />
      <rect x="3" y="13.5" width="7.5" height="7.5" rx="1.5" />
      <rect x="13.5" y="13.5" width="7.5" height="7.5" rx="1.5" />
    </svg>
  );
}

/** Wrench — a tool invocation. */
export function WrenchIcon({ size = 13, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path
        d="M15 6.5a3.5 3.5 0 0 0-4.6 4.3l-5.1 5.1a1.6 1.6 0 0 0 2.3 2.3l5.1-5.1A3.5 3.5 0 0 0 17.5 9l-2 2-2-2 2-2Z"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Shield — a permission gate. */
export function ShieldIcon({
  size = 14,
  strokeWidth = 1.8,
  className,
  filled,
}: IconProps & { filled?: boolean }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      className={className}
    >
      <path d="M12 3 5 6v5c0 4.2 2.8 7.6 7 9 4.2-1.4 7-4.8 7-9V6l-7-3Z" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
    </svg>
  );
}

/** Speech bubble with a question mark — the agent is asking the user something. */
export function QuestionIcon({ size = 14, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path
        d="M20 4H4a1 1 0 0 0-1 1v11a1 1 0 0 0 1 1h3v3.5L11.5 17H20a1 1 0 0 0 1-1V5a1 1 0 0 0-1-1Z"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinejoin="round"
      />
      <path
        d="M9.6 8.6a2.4 2.4 0 0 1 4.4 1.3c0 1.6-2 1.8-2 3.2"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
      />
      <circle cx="12" cy="15.4" r="0.6" fill="currentColor" stroke="currentColor" strokeWidth="0.4" />
    </svg>
  );
}

/** Thumbtack — marks a pinned worktree and the sidebar's "Pinned" section. */
export function PinIcon({ size = 14, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <path d="M12 17v5" />
      <path d="M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.3v.7a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.7a2 2 0 0 0-1.1-1.8l-1.8-.9A2 2 0 0 1 15 10.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
    </svg>
  );
}

/** Glyph for a pipeline stage / agent mode; unknown modes render as general chat. */
export function ModeIcon({ mode, size = 16, strokeWidth = 1.8 }: IconProps & { mode: string }) {
  switch (mode) {
    case "research":
      return (
        <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
          <circle cx="10.5" cy="10.5" r="5.5" stroke="currentColor" strokeWidth={strokeWidth} />
          <path d="m15 15 4 4" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
        </svg>
      );
    case "plan":
      return (
        <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
          <path d="M7 7h11M7 12h11M7 17h11" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
          <path d="m3.5 7 .8.8L6 6M3.5 12l.8.8L6 11M3.5 17l.8.8L6 16" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "implement":
      return (
        <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
          <path d="m9 8-4 4 4 4M15 8l4 4-4 4" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "validate":
      return (
        <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
          <path d="m5 12 4 4 10-10" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "commit":
      return <GitCommitIcon size={size} strokeWidth={strokeWidth} />;
    case "ship":
      return (
        <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
          <path d="M12 3 7 13l5-2 5 2-5-10Z" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
          <path d="M8 17h8M10 21h4" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
        </svg>
      );
    default:
      return (
        <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
          <path d="M5 6.5A2.5 2.5 0 0 1 7.5 4h9A2.5 2.5 0 0 1 19 6.5v6A2.5 2.5 0 0 1 16.5 15H11l-4 4v-4A2.5 2.5 0 0 1 5 12.5v-6Z" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
        </svg>
      );
  }
}

/** Person silhouette — a user/account. */
export function PersonIcon({ size = 14, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <circle cx="12" cy="8" r="3.5" stroke="currentColor" strokeWidth={strokeWidth} />
      <path d="M4 20c0-4.4 3.6-8 8-8s8 3.6 8 8" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

/** Git — branch fork with three commit nodes. */
export function GitBranchIcon({ size = 16, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <circle cx="7" cy="5" r="2.2" stroke="currentColor" strokeWidth={strokeWidth} />
      <circle cx="17" cy="14" r="2.2" stroke="currentColor" strokeWidth={strokeWidth} />
      <circle cx="7" cy="19" r="2.2" stroke="currentColor" strokeWidth={strokeWidth} />
      <path d="M7 7.2V16.8" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
      <path d="M7 9.5C7 9.5 17 9.5 17 11.8" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

/** Obsidian — faceted diamond. */
export function ObsidianIcon({ size = 16, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path d="M12 3L21 9L12 21L3 9L12 3Z" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
      <path d="M3 9L12 13L21 9" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
      <path d="M12 13V21" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

/** Notion — bold N letterform (left stroke, right stroke, diagonal). */
export function NotionIcon({ size = 16, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path
        d="M6 5v14M18 5v14M6 5L18 19"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Anytype — triangle with crossbar (A-shape). */
export function AnytypeIcon({ size = 16, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" className={className}>
      <path d="M12 4L3 20h18L12 4Z" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
      <path d="M7.5 14.5h9" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

/** Archive box — lid over a body. The archive action + the tab-bar tool button. */
export function ArchiveIcon({ size = 16, strokeWidth = 1.5, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <rect x="2" y="2.6" width="12" height="3" rx="0.9" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
      <path
        d="M3 5.6v6.1c0 .77.63 1.4 1.4 1.4h7.2c.77 0 1.4-.63 1.4-1.4V5.6"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path d="M6.3 8.2h3.4" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" />
    </svg>
  );
}

/** Archive box with an up-arrow — the unarchive (restore) action. */
export function UnarchiveIcon({ size = 16, strokeWidth = 1.5, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <rect x="2" y="2.6" width="12" height="3" rx="0.9" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
      <path
        d="M3 5.6v6.1c0 .77.63 1.4 1.4 1.4h7.2c.77 0 1.4-.63 1.4-1.4V5.6"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M8 11.4V7.6M8 7.6 6.4 9.2M8 7.6l1.6 1.6"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Filled play triangle — the Run action. */
export function PlayIcon({ size = 13, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 16 16" fill="currentColor" className={className}>
      <path d="M4.5 3.2c0-.5.54-.82.98-.57l7.2 4.8a.66.66 0 0 1 0 1.14l-7.2 4.8a.66.66 0 0 1-.98-.57Z" />
    </svg>
  );
}

/** Checkmark — a script's last run passed. */
export function CheckIcon({ size = 13, strokeWidth = 1.8, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <path
        d="M3.2 8.4 6.4 11.5 12.8 4.7"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Globe — "Open :PORT" for a detected dev server. */
export function GlobeIcon({ size = 13, strokeWidth = 1.5, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <circle cx="8" cy="8" r="6" stroke="currentColor" strokeWidth={strokeWidth} />
      <path
        d="M2 8h12M8 2c1.8 1.7 1.8 10.3 0 12M8 2c-1.8 1.7-1.8 10.3 0 12"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
      />
    </svg>
  );
}

/** Layout glyph: panel docked to the right edge. */
export function DockRightIcon({ size = 13, strokeWidth = 1.5, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <rect x="2" y="2.5" width="12" height="11" rx="1.8" stroke="currentColor" strokeWidth={strokeWidth} />
      <path d="M10 2.5v11" stroke="currentColor" strokeWidth={strokeWidth} />
      <rect x="10" y="2.5" width="4" height="11" fill="currentColor" opacity="0.35" />
    </svg>
  );
}

/** Layout glyph: panel docked to the bottom edge. */
export function DockBottomIcon({ size = 13, strokeWidth = 1.5, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <rect x="2" y="2.5" width="12" height="11" rx="1.8" stroke="currentColor" strokeWidth={strokeWidth} />
      <path d="M2 9.5h12" stroke="currentColor" strokeWidth={strokeWidth} />
      <rect x="2" y="9.5" width="12" height="4" fill="currentColor" opacity="0.35" />
    </svg>
  );
}

/** A bordered terminal panel — the rail icon for the script/terminal dock,
 *  visually distinct from the Settings gear. */
export function ScriptPanelIcon({ size = 18, strokeWidth = 1.6, className }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <rect x="1.8" y="2.8" width="12.4" height="10.4" rx="2" stroke="currentColor" strokeWidth={strokeWidth} />
      <path
        d="M4.4 6.2 6.6 8.4 4.4 10.6M8.2 10.6h3.4"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
