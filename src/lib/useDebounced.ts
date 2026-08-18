import { useEffect, useState } from "react";

/** The value, held back until it stops changing for `ms`. Shared foundation,
 *  used by the search palette and the Add-project path probe. */
export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const h = setTimeout(() => setV(value), ms);
    return () => clearTimeout(h);
  }, [value, ms]);
  return v;
}
