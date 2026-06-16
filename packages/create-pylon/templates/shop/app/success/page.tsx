import React from "react";
import { Link, type Metadata } from "@pylonsync/react";
import { WRAP } from "@/components/marketing";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: `Thank you — ${siteConfig.brand.name}`,
  description: "Your order is confirmed.",
};

// `app/success/page.tsx` → `/success`. Where Stripe sends the shopper after a
// completed Checkout (the `successUrl` passed to the `checkout` action). The
// order is settled server-side by the signed `stripeWebhook` — this page is
// just the friendly landing, so it doesn't need to read the session id Stripe
// appends. Rendered with the normal marketing chrome (nav + footer).
export default function CheckoutSuccess() {
  return (
    <section className={`${WRAP} flex min-h-[60vh] flex-col items-center justify-center py-20 text-center`}>
      <div className="flex size-12 items-center justify-center rounded-full bg-brand text-xl text-white">✓</div>
      <h1 className="mt-5 text-2xl font-semibold tracking-tight text-zinc-900">Thank you for your order</h1>
      <p className="mx-auto mt-3 max-w-md text-[15px] leading-relaxed text-zinc-500">
        Your payment went through and we&apos;ve emailed your receipt. We&apos;ll be in touch when it ships —
        everything is hand-made, so give us a couple of business days.
      </p>
      <Link
        href="/#shop"
        className="mt-7 inline-flex h-11 items-center rounded-full bg-brand px-6 text-sm font-medium text-white transition-opacity hover:opacity-90"
      >
        Keep shopping
      </Link>
    </section>
  );
}
