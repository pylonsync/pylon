import { useEffect, useState } from "react";
import { db } from "@pylonsync/react";

/**
 * Rows of `entity` for one workspace, live. The server render and the first
 * client paint use `initial` (resolved with `serverData` in the page), so the
 * HTML and the hydrated tree match and nothing flashes empty. After mount the
 * list switches to the synced replica, which updates on every write from any
 * tab or teammate.
 *
 * The entity's read policy already limits the replica to the active
 * workspace; the `orgId` filter drops rows from a previous workspace that
 * can linger for a moment after switching.
 */
export function useLiveRows<T extends { orgId: string }>(
  entity: string,
  initial: T[],
  orgId: string,
): T[] {
  const [hydrated, setHydrated] = useState(false);
  useEffect(() => setHydrated(true), []);
  const { data, loading } = db.useQuery<T>(entity);
  if (!hydrated || loading) return initial;
  return data.filter((row) => row.orgId === orgId);
}
