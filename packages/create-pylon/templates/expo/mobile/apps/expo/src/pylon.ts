import { Platform } from "react-native";
import { init } from "@pylonsync/react-native";

/**
 * The backend URL. `pylon dev` in apps/api listens on 4321. An Android
 * emulator reaches the host machine at 10.0.2.2. Outside development
 * app.config.ts refuses to build without an https:// value.
 */
const configured = process.env.EXPO_PUBLIC_PYLON_BASE_URL?.trim();
export const PYLON_BASE_URL =
  configured ||
  (Platform.OS === "android" ? "http://10.0.2.2:4321" : "http://localhost:4321");

export const APP_NAME = "__APP_NAME_SNAKE__";

let initPromise: Promise<void> | null = null;

/**
 * Boot the sync engine once. Reads the persisted token and replica from
 * AsyncStorage, so a cold launch renders the last known data before the
 * network answers.
 */
export function ensurePylon(): Promise<void> {
  if (!initPromise) {
    initPromise = init({ baseUrl: PYLON_BASE_URL, appName: APP_NAME });
  }
  return initPromise;
}
