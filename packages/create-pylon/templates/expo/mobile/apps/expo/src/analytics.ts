/**
 * Funnel events. Wire `track` to your analytics SDK (PostHog, Amplitude,
 * Mixpanel) in one place; every screen already calls it with the event
 * names below, so the onboarding → paywall → purchase funnel is measurable
 * from the first build.
 */
export type FunnelEvent =
  | "onboarding_started"
  | "onboarding_completed"
  | "paywall_shown"
  | "paywall_dismissed"
  | "purchase_started"
  | "purchase_completed"
  | "purchase_restored"
  | "sign_in_started"
  | "sign_in_completed"
  | "limit_reached"
  | "account_deleted";

export function track(event: FunnelEvent, props: Record<string, unknown> = {}): void {
  if (__DEV__) {
    console.log(`[analytics] ${event}`, props);
  }
  // Example: posthog.capture(event, props);
}
