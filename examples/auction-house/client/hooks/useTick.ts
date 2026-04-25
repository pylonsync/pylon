import { useEffect, useState } from "react";

/**
 * Force a re-render every `ms` milliseconds. Used by countdown
 * displays so they tick down between server updates.
 */
export function useTick(ms = 1000) {
  const [, setN] = useState(0);
  useEffect(() => {
    const t = setInterval(() => setN((n) => n + 1), ms);
    return () => clearInterval(t);
  }, [ms]);
}
