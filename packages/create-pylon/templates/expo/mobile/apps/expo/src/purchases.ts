/**
 * RevenueCat in-app purchases.
 *
 * `react-native-purchases` is a native module: present in a development
 * build and in store builds, absent in Expo Go. Everything here degrades:
 * in Expo Go `available()` is false and the paywall shows a placeholder.
 *
 * Flow: configure(key, appUserID = the Pylon user id) → purchase → the
 * server verifies with RevenueCat (`syncEntitlements`) and writes the
 * RcEntitlement row → it syncs to every device. The app gates on the
 * synced row, never on the SDK's local cache.
 */
import { Platform } from "react-native";
import { callFn } from "@pylonsync/react-native";
import { track } from "./analytics";

type PurchasesModule = typeof import("react-native-purchases").default;
type PurchasesPackage = import("react-native-purchases").PurchasesPackage;

let Purchases: PurchasesModule | null = null;
let configuredFor: string | null = null;

function apiKey(): string | undefined {
  return Platform.OS === "ios"
    ? process.env.EXPO_PUBLIC_REVENUECAT_IOS_KEY
    : process.env.EXPO_PUBLIC_REVENUECAT_ANDROID_KEY;
}

function load(): PurchasesModule | null {
  if (Purchases) return Purchases;
  try {
    // A static import throws at bundle load in Expo Go.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    Purchases = require("react-native-purchases").default as PurchasesModule;
    return Purchases;
  } catch {
    return null;
  }
}

/** True when a key is set and the native module is present. */
export function available(): boolean {
  return Boolean(apiKey()) && load() != null;
}

/** Configure once, then re-identify when the user changes (guest → account). */
export async function configure(userId: string): Promise<boolean> {
  const key = apiKey();
  const mod = load();
  if (!key || !mod) return false;
  try {
    if (configuredFor === null) {
      mod.configure({ apiKey: key, appUserID: userId });
    } else if (configuredFor !== userId) {
      await mod.logIn(userId);
    }
    configuredFor = userId;
    return true;
  } catch {
    return false;
  }
}

export interface Offer {
  id: string;
  title: string;
  price: string;
  /** "monthly" | "annual" | "lifetime" | "weekly" | other */
  period: string;
  /** Free trial length when the store offers one, e.g. "7 days". */
  trial?: string;
  raw: PurchasesPackage;
}

function periodOf(pkg: PurchasesPackage): string {
  switch (pkg.packageType) {
    case "MONTHLY":
      return "monthly";
    case "ANNUAL":
      return "annual";
    case "LIFETIME":
      return "lifetime";
    case "WEEKLY":
      return "weekly";
    default:
      return pkg.identifier;
  }
}

/** The current offering's packages, mapped for the paywall. */
export async function offers(): Promise<Offer[]> {
  const mod = load();
  if (!mod) return [];
  try {
    const offerings = await mod.getOfferings();
    const pkgs = offerings.current?.availablePackages ?? [];
    return pkgs.map((p) => {
      const intro = p.product.introPrice;
      const trial =
        intro && intro.price === 0
          ? `${intro.periodNumberOfUnits} ${intro.periodUnit.toLowerCase()}${intro.periodNumberOfUnits === 1 ? "" : "s"}`
          : undefined;
      return {
        id: p.identifier,
        title: p.product.title,
        price: p.product.priceString,
        period: periodOf(p),
        trial,
        raw: p,
      };
    });
  } catch {
    return [];
  }
}

/** Returns true on purchase, false when the user cancelled. Throws on error. */
export async function purchase(offer: Offer): Promise<boolean> {
  const mod = load();
  if (!mod) return false;
  track("purchase_started", { package: offer.id });
  try {
    await mod.purchasePackage(offer.raw);
  } catch (e) {
    if ((e as { userCancelled?: boolean })?.userCancelled) return false;
    throw e;
  }
  await syncEntitlements();
  track("purchase_completed", { package: offer.id });
  return true;
}

export async function restore(): Promise<boolean> {
  const mod = load();
  if (!mod) return false;
  try {
    await mod.restorePurchases();
  } catch {
    return false;
  }
  await syncEntitlements();
  track("purchase_restored");
  return true;
}

/**
 * Ask the server to re-read this user's entitlements from RevenueCat and
 * write the rows. Makes the purchasing device unlock immediately and
 * covers local dev, where the webhook cannot reach the machine.
 */
export async function syncEntitlements(): Promise<void> {
  try {
    await callFn("syncEntitlements", {});
  } catch {
    // The webhook will land the row; nothing to do here.
  }
}

/** Manage-subscription deep link for the current store. */
export function manageSubscriptionUrl(): string {
  return Platform.OS === "ios"
    ? "https://apps.apple.com/account/subscriptions"
    : "https://play.google.com/store/account/subscriptions";
}
