import { db } from "@pylonsync/react-native";
import { hasEntitlement, type RcEntitlementRow } from "@pylonsync/revenuecat/client";
import {
  hasStripeSubscription,
  type StripeSubscriptionRow,
} from "@pylonsync/stripe/entitlement";

export const PRO = "pro";

/**
 * Live Pro status, from whichever store the subscriber actually paid.
 *
 * Both are read, because a subscriber has exactly one of them: a purchase in
 * the App Store or Play Store arrives as an RcEntitlement row, and one made
 * on the website arrives as a StripeSubscription row. Reading only the first
 * is how someone subscribes on the web and finds the app still locked — and
 * cross-platform access is the reason to run both in the first place.
 *
 * An app using only one of the two plugins simply has no rows of the other
 * kind, and the query costs nothing.
 *
 * Both plugin entry points here are the browser/React Native ones. The
 * package roots are server code and must not be imported from the app.
 */
export function usePro(): { pro: boolean; loading: boolean } {
  const rc = db.useQuery<RcEntitlementRow>("RcEntitlement", {});
  const stripe = db.useQuery<StripeSubscriptionRow>("StripeSubscription", {});

  // Access as soon as either source says so; still loading only while both
  // are outstanding, so a subscriber is not shown a paywall mid-boot.
  const pro =
    hasEntitlement(rc.data ?? [], PRO) || hasStripeSubscription(stripe.data ?? []);
  return { pro, loading: rc.loading && stripe.loading };
}
