/**
 * Everything the marketing and legal pages read. This is the one file to
 * edit when you rename the app, add store links, or fill in your company
 * details.
 *
 * It is a plain module, not env vars, on purpose: SSR pages are bundled for
 * hydration and the bundler rewrites `process.env.*`, so page code cannot
 * read arbitrary env at render time. Server-only route modules
 * (app/sitemap.ts, app/robots.ts) can, and do.
 */

export const site = {
  /** Shown in the header, the page titles, and the legal pages. */
  name: "__APP_NAME__",
  /** The hero headline. One sentence, what the app does for the person. */
  tagline: "Your notes, on every device, the moment you write them",
  /** The line under the headline, and the meta description. */
  description:
    "__APP_NAME__ keeps your notes in sync across your phone, tablet, and anything else you sign in on. It works offline and catches up when you are back.",

  /**
   * The public URL of this site once deployed, with no trailing slash.
   * `pylon deploy` prints the host. Used for canonical links and the
   * sitemap, and by the mobile app when it opens the legal pages.
   */
  url: "",

  /** The legal entity that publishes the app. Appears on both legal pages. */
  company: "",
  /** Where support email and legal notices go. */
  supportEmail: "",
  /** Postal address. Apple asks for one on the privacy policy. */
  address: "",
  /** The law that governs the terms, e.g. "the State of Texas, USA". */
  governingLaw: "",
  /** Date the current legal text took effect, as YYYY-MM-DD. */
  legalEffective: "2026-01-01",

  /** Store links. Empty renders a "coming soon" state instead of a dead link. */
  appStoreUrl: "",
  playStoreUrl: "",

  /** The three or four things worth saying on the landing page. */
  features: [
    {
      title: "Write now, sync later",
      body: "Notes save on your device first, so the app never waits on a network. They reach your other devices as soon as there is one.",
    },
    {
      title: "Start without an account",
      body: "Open the app and use it. Sign in when you want your notes on a second device, and everything you already wrote comes with you.",
    },
    {
      title: "Yours to delete",
      body: "Delete your account in Settings and your notes go with it. No support ticket, no waiting.",
    },
  ],

  /** Free tier and subscription, kept honest with what the app enforces. */
  pricing: {
    freeLimit: 10,
    proBlurb: "Unlimited notes and everything shipped next.",
  },

  faq: [
    {
      q: "Do I need an account?",
      a: "No. The app works straight away. Signing in keeps your notes if you switch phones.",
    },
    {
      q: "How do I cancel a subscription?",
      a: "Subscriptions are billed by Apple or Google, so cancel from your App Store or Play Store account. Access lasts until the end of the period you paid for.",
    },
    {
      q: "What happens to my notes if I delete my account?",
      a: "They are deleted with it. Settings has the button, and it takes effect immediately.",
    },
  ],
} as const;

/**
 * Config the site cannot honestly render without. The header shows a notice
 * while any of these are blank, so an unfinished site cannot quietly go live
 * with placeholder legal text.
 */
export function missingSiteConfig(): string[] {
  const missing: string[] = [];
  if (!site.url) missing.push("url");
  if (!site.company) missing.push("company");
  if (!site.supportEmail) missing.push("supportEmail");
  if (!site.address) missing.push("address");
  if (!site.governingLaw) missing.push("governingLaw");
  return missing;
}
