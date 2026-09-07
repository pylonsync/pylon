import { db } from "@pylonsync/react-native";
import { hasEntitlement, type RcEntitlementRow } from "@pylonsync/revenuecat/client";

export const PRO = "pro";

/**
 * Live Pro status from the synced RcEntitlement rows. Updates the moment
 * the server writes the row after a purchase, on every device.
 *
 * `@pylonsync/revenuecat/client` is the browser/React Native entry of the
 * plugin. The package root is server code and must not be imported here.
 */
export function usePro(): { pro: boolean; loading: boolean } {
  const { data, loading } = db.useQuery<RcEntitlementRow>("RcEntitlement", {});
  return { pro: hasEntitlement(data ?? [], PRO), loading };
}
