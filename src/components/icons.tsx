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

export function DocIcon({ size = 12, strokeWidth = 2 }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M6 3h8l4 4v14H6V3Z" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
      <path d="M14 3v4h4" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
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

/** Filled square — interrupt / stop the running turn. */
export function StopIcon({ size = 14 }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <rect x="6" y="6" width="12" height="12" rx="2" />
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
export function ShieldIcon({ size = 14, strokeWidth = 1.8 }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M12 3 5 6v5c0 4.2 2.8 7.6 7 9 4.2-1.4 7-4.8 7-9V6l-7-3Z" stroke="currentColor" strokeWidth={strokeWidth} strokeLinejoin="round" />
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
