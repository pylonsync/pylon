import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { Check } from "lucide-react";
import { WRAP, BUTTON_DARK } from "@/components/marketing";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: `Thank you | ${siteConfig.brand.name}`,
  description: "Your order is confirmed.",
};

// `app/(marketing)/success/page.tsx` → `/success`. Two ways in:
//   • Stripe sends the shopper here after a completed Checkout (the
//     `successUrl` passed to the `checkout` action). The signed `stripeWebhook`
//     settles the order, so this page doesn't read the session id Stripe appends.
//   • With no Stripe key, the cart sends the shopper here with `?order=reserved`
//     (plus `&sold=` naming any line that sold out during checkout).
// The page shows no order details: Order rows hold PII and are owner-only.
export default function CheckoutSuccess({ searchParams }: PageProps) {
  const reserved = searchParams.order === "reserved";
  const sold = (searchParams.sold ?? "").split(",").filter(Boolean);
  const { checkout, brand } = siteConfig;

  const steps = reserved
    ? ["Order reserved", "Payment link sent by email", "Packed and shipped from Dallas"]
    : ["Payment received", "Packed in the studio", "Shipped from Dallas"];

  return (
    <section className={`${WRAP} grid min-h-[72vh] items-center gap-14 py-16 md:grid-cols-[1.4fr_1fr] md:gap-20 md:py-20`}>
      <div>
        <h1 className="rise font-display text-[3rem] font-light leading-[1] tracking-[-0.03em] text-ink sm:text-[4.25rem]">
          {reserved ? "Your order is reserved." : "Thank you for your order."}
        </h1>
        <p
          className="rise mt-6 max-w-[46ch] text-[17px] leading-[1.65] text-ink-2"
          style={{ animationDelay: "100ms" }}
        >
          {reserved ? checkout.reservedMessage : checkout.paidMessage}
        </p>
        {sold.length > 0 ? (
          <p className="mt-4 max-w-[46ch] text-[14.5px] leading-relaxed text-brand">
            {sold.join(", ")} sold out before we could hold {sold.length === 1 ? "it" : "them"}, so{" "}
            {sold.length === 1 ? "it is" : "they are"} not in this order.
          </p>
        ) : null}

        <ol className="rise mt-10 max-w-md border-t border-ink/15" style={{ animationDelay: "200ms" }}>
          {steps.map((step, i) => (
            <li key={step} className="flex items-center gap-4 border-b border-ink/15 py-4 text-[15px]">
              <span
                className={
                  "flex size-7 shrink-0 items-center justify-center rounded-full text-[12px] tabular-nums " +
                  (i === 0 ? "bg-ink text-warm-white" : "border border-ink/20 text-ink-2")
                }
              >
                {i === 0 ? <Check className="size-3.5" strokeWidth={2} /> : i + 1}
              </span>
              <span className={i === 0 ? "font-medium text-ink" : "text-ink-2"}>{step}</span>
            </li>
          ))}
        </ol>

        <div className="rise mt-10 flex flex-wrap items-center gap-5" style={{ animationDelay: "300ms" }}>
          <Link href="/#shop" className={BUTTON_DARK}>
            Back to the shop
          </Link>
          <a
            href={`mailto:${brand.email}`}
            className="text-[14px] text-ink-2 underline decoration-ink/20 underline-offset-4 transition-colors hover:text-ink hover:decoration-ink"
          >
            Questions? {brand.email}
          </a>
        </div>
      </div>

      <div className="relative aspect-[4/5] overflow-hidden bg-stone">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src={checkout.image}
          alt=""
          className="settle size-full object-cover"
        />
      </div>
    </section>
  );
}
