import { useEffect, useState } from "react";

// Re-renders often enough for a relative timestamp: every 10s under a minute, backing off with age.
export function useTickingNow(epochMs: number | null): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (epochMs === null) return;
    let id: number;
    const schedule = () => {
      const ageMs = Date.now() - epochMs;
      const delay = ageMs < 60_000 ? 10_000 : ageMs < 3_600_000 ? 30_000 : 300_000;
      id = window.setTimeout(() => {
        setNow(Date.now());
        schedule();
      }, delay);
    };
    schedule();
    return () => window.clearTimeout(id);
  }, [epochMs]);
  return now;
}
