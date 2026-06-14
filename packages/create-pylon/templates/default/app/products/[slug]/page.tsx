import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import {
  WRAP,
  Divider,
  Eyebrow,
  FeatureGrid,
  PrimaryButton,
  GhostLink,
  Shot,
} from "@/components/marketing";
import { PRODUCTS, productBySlug } from "@/lib/products";

// Per-product SEO. `generateMetadata` runs on the server with the resolved
// route params, so each /products/<slug> page gets its own <title>/<meta> in
// the HTML — fully indexable, no client work.
export function generateMetadata({ params }: PageProps): Metadata {
  const product = productBySlug(params.slug);
  if (!product) return { title: "Not found — Acme", robots: "noindex" };
  return {
    title: `${product.title} — Acme`,
    description: product.summary,
  };
}

// `app/products/[slug]/page.tsx` → `/products/:slug`. One template, every
// product. The slug is resolved from the URL during the server render; an
// unknown slug becomes a real 404 (via `response.notFound`) before any HTML is
// sent. Add a product in lib/products.ts and its page exists automatically.
export default function ProductPage({ params, auth, response }: PageProps) {
  const product = productBySlug(params.slug);
  if (!product) {
    response.notFound();
    return null;
  }

  const signedIn = Boolean(auth.user_id);
  const primaryHref = signedIn ? "/dashboard" : "/signup";
  const others = PRODUCTS.filter((p) => p.slug !== product.slug);

  return (
    <div className="bg-white text-zinc-900">
      {/* ============================ HERO ============================ */}
      <section className={`${WRAP} pt-16 pb-16 sm:pt-20`}>
        <Link
          href="/#product"
          className="text-[13px] text-zinc-500 transition-colors hover:text-zinc-900"
        >
          ← All products
        </Link>
        <div className="mt-6 flex items-center gap-3">
          <span className="flex size-9 items-center justify-center rounded-lg bg-brand-soft text-brand">
            {product.icon}
          </span>
          <Eyebrow>{product.eyebrow}</Eyebrow>
        </div>
        <h1 className="mt-5 max-w-2xl text-balance text-[2.5rem] font-semibold leading-[1.05] tracking-[-0.02em] sm:text-[3rem]">
          {product.headline}
        </h1>
        <p className="mt-6 max-w-xl text-[17px] leading-relaxed text-zinc-500">
          {product.summary}
        </p>
        <div className="mt-8 flex flex-wrap items-center gap-4">
          <PrimaryButton href={primaryHref}>
            {signedIn ? "Open dashboard" : "Get started"}
          </PrimaryButton>
          <GhostLink href="/#pricing">See pricing →</GhostLink>
        </div>

        <div className="mt-16">
          <Shot url={product.mockupUrl} label={product.mockupLabel} />
        </div>
      </section>

      {/* ========================= FEATURES ========================= */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <Eyebrow>What you get</Eyebrow>
        <h2 className="mt-4 max-w-xl text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          {product.title}, end to end.
        </h2>
        <FeatureGrid className="mt-12" items={product.features} />
      </section>

      {/* ==================== EXPLORE OTHER ======================== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <Eyebrow>More from Acme</Eyebrow>
        <h2 className="mt-4 text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          Explore the rest of the platform.
        </h2>
        <div className="mt-12 grid gap-5 sm:grid-cols-2 lg:grid-cols-4">
          {others.map((p) => (
            <Link
              key={p.slug}
              href={`/products/${p.slug}`}
              className="group rounded-2xl border border-zinc-200 bg-paper p-6 transition-colors hover:border-zinc-300 hover:bg-white"
            >
              <span className="flex size-9 items-center justify-center rounded-lg bg-brand-soft text-brand">
                {p.icon}
              </span>
              <h3 className="mt-4 text-[15px] font-semibold">{p.title}</h3>
              <p className="mt-1.5 text-[13px] leading-relaxed text-zinc-500">
                {p.tagline}
              </p>
              <span className="mt-3 inline-block text-[13px] font-medium text-brand">
                Explore →
              </span>
            </Link>
          ))}
        </div>
      </section>

      {/* =========================== CTA ========================== */}
      <Divider />
      <section className={`${WRAP} py-24`}>
        <h2 className="max-w-xl text-balance text-[2.25rem] font-semibold leading-[1.05] tracking-[-0.02em] sm:text-[2.75rem]">
          Get your team on {product.title}.
        </h2>
        <p className="mt-6 max-w-lg text-[16px] leading-relaxed text-zinc-500">
          Start free and have your first workspace running in under a minute.
        </p>
        <div className="mt-8">
          <PrimaryButton href={primaryHref}>
            {signedIn ? "Open dashboard" : "Start building with Acme"}
          </PrimaryButton>
        </div>
        <p className="mt-4 text-[12px] text-zinc-400">
          Free to start · No credit card · Cancel anytime
        </p>
      </section>
    </div>
  );
}
