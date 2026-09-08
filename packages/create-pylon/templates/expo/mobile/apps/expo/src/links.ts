/**
 * Where the app sends people for legal text and help.
 *
 * The backend in `apps/api` serves a website as well as the API, so these
 * pages already exist on whatever host `EXPO_PUBLIC_PYLON_BASE_URL` points
 * at. That is what makes the App Store and Play Store requirements
 * satisfiable straight from a scaffold: both stores refuse a submission
 * without a reachable privacy policy URL.
 *
 * Set the EXPO_PUBLIC_* values only if you host the marketing site somewhere
 * else, on your own domain for example.
 */
import { PYLON_BASE_URL } from "./pylon";

const siteOrigin = (
  process.env.EXPO_PUBLIC_SITE_URL || PYLON_BASE_URL
).replace(/\/+$/, "");

export const PRIVACY_URL =
  process.env.EXPO_PUBLIC_PRIVACY_URL || `${siteOrigin}/privacy`;

export const TERMS_URL =
  process.env.EXPO_PUBLIC_TERMS_URL || `${siteOrigin}/terms`;

export const SUPPORT_URL =
  process.env.EXPO_PUBLIC_SUPPORT_URL || `${siteOrigin}/support`;

/** Optional. Settings shows a "Contact support" row when it is set. */
export const SUPPORT_EMAIL = process.env.EXPO_PUBLIC_SUPPORT_EMAIL;
