import React from "react";
import { type Metadata } from "@pylonsync/react";
import { site } from "@/lib/site";

export const metadata: Metadata = {
  title: `${site.name} — ${site.tagline}`,
  description: site.description,
};

/**
 * The landing page, served at `/` by the same Pylon server that answers the
 * mobile app's API calls. That is why the legal pages below are a real URL
 * the moment you deploy: the App Store and Play Store both refuse a
 * submission without one.
 *
 * Replace the copy through `lib/site.ts`. The sections themselves are here.
 */
export default function LandingPage() {
  return (
    <>
      <Hero />
      <Features />
      <Pricing />
      <Faq />
      <GetTheApp />
    </>
  );
}

function Hero() {
  return (
    <section className="mx-auto max-w-5xl px-6 pb-16 pt-20 sm:pt-28">
      <div className="grid items-center gap-14 lg:grid-cols-[1.05fr_0.95fr]">
        <div>
          <h1 className="text-4xl font-semibold leading-[1.08] tracking-[-0.02em] sm:text-5xl">
            {site.tagline}
          </h1>
          <p className="mt-5 max-w-xl text-[16.5px] leading-relaxed text-ink-muted">
            {site.description}
          </p>
          <div className="mt-8">
            <StoreButtons />
          </div>
          <p className="mt-4 text-[13px] text-ink-muted">
            Free for your first {site.pricing.freeLimit} notes. No account
            needed to start.
          </p>
        </div>
        <PhoneShot />
      </div>
    </section>
  );
}

/**
 * A real link once the store URL is in `lib/site.ts`, and an honest "not yet"
 * until then. A dead link on a launch page is worse than saying nothing.
 */
function StoreButtons() {
  return (
    <div className="flex flex-wrap gap-3">
      <StoreButton
        href={site.appStoreUrl}
        label="Download on the App Store"
        pending="Coming to the App Store"
      />
      <StoreButton
        href={site.playStoreUrl}
        label="Get it on Google Play"
        pending="Coming to Google Play"
      />
    </div>
  );
}

function StoreButton({
  href,
  label,
  pending,
}: {
  href: string;
  label: string;
  pending: string;
}) {
  if (!href) {
    return (
      <span className="rounded-xl border border-dashed border-line px-5 py-3 text-[14px] text-ink-muted">
        {pending}
      </span>
    );
  }
  return (
    <a
      href={href}
      className="rounded-xl bg-ink px-5 py-3 text-[14px] font-medium text-surface transition-opacity hover:opacity-90"
    >
      {label}
    </a>
  );
}

/** Placeholder, marked as one. Swap it for a real screenshot before launch. */
function PhoneShot() {
  return (
    <div className="mx-auto w-full max-w-[280px]">
      <div className="rounded-[2.25rem] border border-line bg-surface-soft p-3 shadow-[0_30px_80px_-40px_rgba(0,0,0,0.45)]">
        <div className="flex aspect-[9/19.5] flex-col items-center justify-center gap-3 rounded-[1.75rem] border-2 border-dashed border-line px-6 text-center">
          <svg
            width="26"
            height="26"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            className="text-ink-muted"
            aria-hidden="true"
          >
            <rect x="3" y="5" width="18" height="14" rx="2" />
            <circle cx="8.5" cy="10" r="1.5" />
            <path d="m21 16-5-5-6 6-3-3-4 4" />
          </svg>
          <p className="text-[13px] font-medium">App screenshot</p>
          <p className="text-[12px] leading-snug text-ink-muted">
            Drop a 1290×2796 screenshot in here. The component is
            <code className="mx-1 font-mono">PhoneShot</code>
            in this file.
          </p>
        </div>
      </div>
    </div>
  );
}

function Features() {
  return (
    <section id="features" className="border-t border-line bg-surface-soft">
      <div className="mx-auto max-w-5xl px-6 py-20">
        <h2 className="max-w-2xl text-2xl font-semibold tracking-[-0.015em] sm:text-3xl">
          What it does
        </h2>
        <div className="mt-10 grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
          {site.features.map((feature) => (
            <div
              key={feature.title}
              className="rounded-2xl border border-line bg-surface p-6"
            >
              <h3 className="text-[15px] font-semibold">{feature.title}</h3>
              <p className="mt-2 text-[14px] leading-relaxed text-ink-muted">
                {feature.body}
              </p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function Pricing() {
  return (
    <section className="mx-auto max-w-5xl px-6 py-20">
      <h2 className="text-2xl font-semibold tracking-[-0.015em] sm:text-3xl">
        What it costs
      </h2>
      <div className="mt-10 grid gap-6 sm:grid-cols-2">
        <div className="rounded-2xl border border-line p-7">
          <h3 className="text-[15px] font-semibold">Free</h3>
          <p className="mt-2 text-[14px] leading-relaxed text-ink-muted">
            Up to {site.pricing.freeLimit} notes, synced across your devices.
            Nothing expires.
          </p>
        </div>
        <div className="rounded-2xl border border-brand bg-brand-soft p-7">
          <h3 className="text-[15px] font-semibold">Pro</h3>
          <p className="mt-2 text-[14px] leading-relaxed text-ink-muted">
            {site.pricing.proBlurb} Prices are set per country in the store and
            shown in the app before you buy.
          </p>
        </div>
      </div>
    </section>
  );
}

function Faq() {
  return (
    <section id="faq" className="border-t border-line bg-surface-soft">
      <div className="mx-auto max-w-3xl px-6 py-20">
        <h2 className="text-2xl font-semibold tracking-[-0.015em] sm:text-3xl">
          Questions
        </h2>
        <dl className="mt-10 divide-y divide-line border-y border-line">
          {site.faq.map((item) => (
            <div key={item.q} className="py-5">
              <dt className="text-[15px] font-medium">{item.q}</dt>
              <dd className="mt-2 text-[14px] leading-relaxed text-ink-muted">
                {item.a}
              </dd>
            </div>
          ))}
        </dl>
      </div>
    </section>
  );
}

function GetTheApp() {
  return (
    <section id="get" className="mx-auto max-w-5xl px-6 py-20 text-center">
      <h2 className="text-2xl font-semibold tracking-[-0.015em] sm:text-3xl">
        Get {site.name}
      </h2>
      <p className="mx-auto mt-3 max-w-md text-[15px] leading-relaxed text-ink-muted">
        Free to start, on iPhone and Android.
      </p>
      <div className="mt-8 flex justify-center">
        <StoreButtons />
      </div>
    </section>
  );
}
