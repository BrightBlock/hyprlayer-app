/**
 * A small activity dot summarising a whole workstream's live session state for
 * list/sidebar rows: blue pulse = a turn is running, amber pulse = blocked on
 * you (permission/question), red = a session errored, green = a turn finished
 * and wants review. Nothing renders when there's no activity to flag, so a quiet
 * workstream stays visually quiet. See `useWorkstreamActivity` for the fold.
 */
import { useWorkstreamActivity, type WorkstreamActivity } from "./sessionStore";

type DotAnim = "ping" | "work" | "none";

const ACTIVITY_STYLE: Record<
  Exclude<WorkstreamActivity, "idle">,
  { color: string; anim: DotAnim; label: string }
> = {
  // Radar ping reads as "alert" — fits a session blocked on you.
  "needs-you": { color: "bg-warn", anim: "ping", label: "Waiting on you" },
  // Soft breathing reads as "live, ongoing work" — same language as the tab glow.
  working: { color: "bg-working text-working", anim: "work", label: "Working" },
  error: { color: "bg-bad", anim: "none", label: "Errored" },
  review: { color: "bg-ok", anim: "none", label: "Turn done — ready for review" },
  // Violet breathing = "passive background work" (turn done, but shells running).
  // Distinct hue from blue "working" so the two are visually separable.
  background: { color: "bg-background text-background", anim: "work", label: "Background tasks running" },
};

/**
 * Leading/trailing status dot for a workstream row. Idle is invisible by default
 * (`idle="hide"`); pass `idle="dot"` to render a muted placeholder dot instead,
 * so a list reads as an even column of status dots — the sidebar tree's look.
 */
export function WorkstreamActivityDot({
  workstreamId,
  idle = "hide",
}: {
  workstreamId: string;
  idle?: "hide" | "dot";
}) {
  const activity = useWorkstreamActivity(workstreamId);
  if (activity === "idle") {
    if (idle === "dot") {
      return (
        <span
          className="flex h-1.5 w-1.5 shrink-0 items-center justify-center"
          aria-hidden="true"
        >
          <span className="h-1.5 w-1.5 rounded-full bg-ink-dim/40" />
        </span>
      );
    }
    return null;
  }
  const { color, anim, label } = ACTIVITY_STYLE[activity];
  return (
    <span
      className="relative flex h-1.5 w-1.5 shrink-0 items-center justify-center"
      title={label}
      role="status"
      aria-label={label}
    >
      {anim === "ping" && (
        <span
          className={`absolute inline-flex h-full w-full animate-ping rounded-full opacity-60 ${color}`}
          aria-hidden="true"
        />
      )}
      <span
        className={`relative inline-flex h-1.5 w-1.5 rounded-full ${color} ${
          anim === "work" ? "animate-work-pulse" : ""
        }`}
        aria-hidden="true"
      />
    </span>
  );
}
