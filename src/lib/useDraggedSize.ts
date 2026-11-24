import { useState } from "react";

// Holds a separator's size locally while it moves and stores it once, on release,
// so a drag writes the persisted store one time instead of on every pointer move.
export function useDraggedSize(stored: number, store: (size: number) => void) {
  const [live, setLive] = useState<number | null>(null);
  return {
    size: live ?? stored,
    onResize: setLive,
    onEnd: () =>
      setLive((current) => {
        if (current !== null) store(current);
        return null;
      }),
  };
}
