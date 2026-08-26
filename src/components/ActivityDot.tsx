import type { WorkstreamActivity } from "@/lib/domain/activity";

type DotAnim = "ping" | "work" | "none";

const ACTIVITY_STYLE: Record<
  Exclude<WorkstreamActivity, "idle">,
  { color: string; anim: DotAnim; label: string }
> = {
  "needs-you": { color: "bg-warn", anim: "ping", label: "Waiting on you" },
  working: { color: "bg-working text-working", anim: "work", label: "Working" },
  error: { color: "bg-bad", anim: "none", label: "Errored" },
  review: { color: "bg-ok", anim: "none", label: "Turn done, ready for review" },
  background: { color: "bg-background text-background", anim: "work", label: "Background tasks running" },
};

export function ActivityDot({
  activity,
  idle = "hide",
}: {
  activity: WorkstreamActivity;
  idle?: "hide" | "dot";
}) {
  if (activity === "idle") {
    if (idle === "dot") {
      return (
        <span className="flex h-1.5 w-1.5 shrink-0 items-center justify-center" aria-hidden="true">
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
