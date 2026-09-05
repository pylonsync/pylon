import { db } from "@pylonsync/react-native";
import { hasEntitlement } from "@pylonsync/revenuecat";

export const PRO = "pro";

interface RcEntitlementRow {
  id: string;
  userId: string;
  entitlement: string;
  status: string;
  expiresAt?: string | null;
}

/**
 * Live Pro status from the synced RcEntitlement rows. Updates the moment
 * the server writes the row after a purchase, on every device.
 */
export function usePro(): { pro: boolean; loading: boolean } {
  const { data, loading } = db.useQuery<RcEntitlementRow>("RcEntitlement", {});
  return { pro: hasEntitlement(data ?? [], PRO), loading };
}
