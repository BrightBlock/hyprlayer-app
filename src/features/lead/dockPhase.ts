export type DockPhase = "closed" | "growing" | "open" | "fading" | "shrinking";

export type DockEvent = "expand" | "minimize" | "settled" | "restored" | "dropped";

export function nextDockPhase(phase: DockPhase, event: DockEvent): DockPhase {
  switch (event) {
    case "expand":
      return phase === "closed" ? "growing" : phase;
    case "minimize":
      return phase === "open" || phase === "growing" ? "fading" : phase;
    case "settled":
      return phase === "growing" ? "open" : phase === "fading" ? "shrinking" : phase === "shrinking" ? "closed" : phase;
    case "restored":
      return phase === "closed" ? "open" : phase;
    case "dropped":
      return phase === "open" ? "closed" : phase;
  }
}
