import type { ConfigContext, ExpoConfig } from "expo/config";

/**
 * App config. The values that change per environment come from env vars
 * so one file serves development builds, TestFlight, and the store:
 *
 *   APP_BUNDLE_ID                reverse-DNS id (iOS bundle id + Android package)
 *   APP_SCHEME                   deep-link scheme, e.g. "__APP_NAME_KEBAB__"
 *   EAS_PROJECT_ID               from `eas init` (printed once, then stable)
 *   EXPO_PUBLIC_PYLON_BASE_URL   the backend; https:// outside development
 *   APP_VARIANT                  "development" | "preview" | "production" (set by eas.json)
 *
 * A development build gets a ".dev" suffix so it installs alongside the
 * store app on the same phone.
 */
const variant = process.env.APP_VARIANT ?? "development";
const baseBundleId = process.env.APP_BUNDLE_ID ?? "com.example.__APP_NAME_SNAKE__";
const bundleId = variant === "development" ? `${baseBundleId}.dev` : baseBundleId;
const name = variant === "production" ? "__APP_NAME__" : `__APP_NAME__ (${variant})`;

// A preview or store build with placeholder values would install and
// then fail at first launch. Refuse to build instead.
if (variant !== "development") {
  const problems: string[] = [];
  if (!process.env.APP_BUNDLE_ID) {
    problems.push("APP_BUNDLE_ID is unset; the com.example placeholder cannot ship");
  }
  if (!process.env.EAS_PROJECT_ID) {
    problems.push("EAS_PROJECT_ID is unset; run `eas init` and copy the id");
  }
  if (!/^https:\/\//.test(process.env.EXPO_PUBLIC_PYLON_BASE_URL ?? "")) {
    problems.push("EXPO_PUBLIC_PYLON_BASE_URL must be the deployed https:// backend");
  }
  if (problems.length > 0) {
    throw new Error(
      `The ${variant} build is not configured:\n  - ${problems.join("\n  - ")}\n` +
        `Set these under build.${variant}.env in eas.json, or export them before building.`,
    );
  }
}

export default ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name,
  slug: "__APP_NAME_KEBAB__",
  version: "1.0.0",
  scheme: process.env.APP_SCHEME ?? "__APP_NAME_KEBAB__",
  orientation: "portrait",
  icon: "./assets/icon.png",
  userInterfaceStyle: "automatic",
  ios: {
    bundleIdentifier: bundleId,
    supportsTablet: false,
    // Sign in with Apple. Also enable the capability on the App ID in
    // the Apple Developer portal (EAS does this for you on first build).
    usesAppleSignIn: true,
    infoPlist: {
      // Local dev talks to http://localhost:4321.
      NSAppTransportSecurity: { NSAllowsLocalNetworking: true },
      // App Store Connect asks; this app uses only standard TLS.
      ITSAppUsesNonExemptEncryption: false,
    },
  },
  android: {
    package: bundleId,
    adaptiveIcon: {
      foregroundImage: "./assets/adaptive-icon.png",
      backgroundColor: "#18181b",
    },
  },
  plugins: [
    "expo-router",
    "expo-apple-authentication",
    [
      "expo-splash-screen",
      { image: "./assets/splash-icon.png", resizeMode: "contain", backgroundColor: "#18181b" },
    ],
    [
      "expo-build-properties",
      {
        // The floors for Expo SDK 57 / React Native 0.86. expo-build-properties
        // rejects a lower iOS target; Android below 24 does not build.
        ios: { deploymentTarget: "16.4" },
        // Cleartext lets a dev/preview build reach http://localhost:4321.
        android: { minSdkVersion: 24, usesCleartextTraffic: variant !== "production" },
      },
    ],
  ],
  experiments: { typedRoutes: true },
  extra: {
    eas: { projectId: process.env.EAS_PROJECT_ID },
    variant,
  },
});
