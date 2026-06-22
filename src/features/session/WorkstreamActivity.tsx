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
  working: { color: "bg-sky-400 text-sky-400", anim: "work", label: "Working" },
  error: { color: "bg-bad", anim: "none", label: "Errored" },
  review: { color: "bg-ok", anim: "none", label: "Turn done — ready for review" },
  // Violet breathing = "passive background work" (turn done, but shells running).
  // Distinct hue from blue "working" so the two are visually separable.
  background: { color: "bg-violet-400 text-violet-400", anim: "work", label: "Background tasks running" },
};

/** Leading/trailing status dot for a workstream row. Renders null while idle. */
export function WorkstreamActivityDot({ workstreamId }: { workstreamId: string }) {
  const activity = useWorkstreamActivity(workstreamId);
  if (activity === "idle") return null;
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
