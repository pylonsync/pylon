import AsyncStorage from "@react-native-async-storage/async-storage";
import { APP_NAME } from "./pylon";

/** Small per-device flags that drive the first-run flow. */
const KEYS = {
  onboardingDone: `${APP_NAME}:onboarding_done`,
  paywallSeenAt: `${APP_NAME}:paywall_seen_at`,
} as const;

export async function getOnboardingDone(): Promise<boolean> {
  return (await AsyncStorage.getItem(KEYS.onboardingDone)) === "1";
}

export async function setOnboardingDone(): Promise<void> {
  await AsyncStorage.setItem(KEYS.onboardingDone, "1");
}

/** When the post-onboarding paywall was last shown, or null. */
export async function getPaywallSeenAt(): Promise<number | null> {
  const raw = await AsyncStorage.getItem(KEYS.paywallSeenAt);
  return raw ? Number(raw) : null;
}

export async function setPaywallSeenAt(at: number = Date.now()): Promise<void> {
  await AsyncStorage.setItem(KEYS.paywallSeenAt, String(at));
}

/** Test helper and "Reset onboarding" in Settings. */
export async function resetFlags(): Promise<void> {
  await AsyncStorage.multiRemove(Object.values(KEYS));
}
