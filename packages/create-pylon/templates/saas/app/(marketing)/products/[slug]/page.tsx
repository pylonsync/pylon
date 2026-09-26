import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { ArrowRight } from "lucide-react";
import { WRAP, FeatureList, PrimaryButton, ProductShot, SecondaryButton } from "@/components/marketing";
import { PRODUCTS, productBySlug } from "@/lib/products";
import { siteConfig } from "@/lib/site.config";

// Per-product SEO. `generateMetadata` runs on the server with the resolved
// route params, so each /products/<slug> page gets its own <title>/<meta>.
export function generateMetadata({ params }: PageProps): Metadata {
  const product = productBySlug(params.slug);
  if (!product) return { title: `Not found — ${siteConfig.brand.name}`, robots: "noindex" };
  return {
    title: `${product.title} — ${siteConfig.brand.name}`,
    description: product.summary,
  };
}

// `/products/:slug` — one template, every product. An unknown slug becomes a
// real 404 (via `response.notFound`) before any HTML is sent. Add a product
// in lib/site.config.ts and its page exists.
export default function ProductPage({ params, auth, response }: PageProps) {
  const product = productBySlug(params.slug);
  if (!product) {
    response.notFound();
    return null;
  }
  const signedIn = Boolean(auth.user_id);
  const others = PRODUCTS.filter((p) => p.slug !== product.slug);

  return (
    <div className="bg-white text-zinc-900">
      <section className={`${WRAP} pb-16 pt-16 sm:pt-24`}>
        <span className="flex size-10 items-center justify-center rounded-xl bg-white text-zinc-700 shadow-[0_0_0_1px_rgba(0,0,0,0.07),0_2px_4px_-1px_rgba(0,0,0,0.06)]">
          <product.icon className="size-5" strokeWidth={1.75} />
        </span>
        <h1 className="mt-6 max-w-3xl text-balance text-[2.5rem] font-semibold leading-[1.05] tracking-[-0.035em] text-zinc-950 sm:text-[3.25rem]">
          {product.headline}
        </h1>
        <p className="mt-5 max-w-xl text-[17px] leading-relaxed text-zinc-500">{product.summary}</p>
        <div className="mt-8 flex flex-wrap items-center gap-3">
          <PrimaryButton href={signedIn ? "/dashboard" : "/signup"}>
            {signedIn ? "Open dashboard" : "Get started"}
          </PrimaryButton>
          <SecondaryButton href="/pricing">See pricing</SecondaryButton>
        </div>
        <div className="mt-14">
          <ProductShot src={product.screenshot} alt={product.screenshotAlt} priority />
        </div>
      </section>

      <section className={`${WRAP} pb-20`}>
        <div className="max-w-3xl">
          <FeatureList items={product.features} />
        </div>
      </section>

      <section className="border-t border-zinc-200/70 bg-zinc-50/70">
        <div className={`${WRAP} py-16`}>
          <h2 className="text-[1.5rem] font-semibold tracking-[-0.02em] text-zinc-950">More in {siteConfig.brand.name}</h2>
          <div className="mt-8 grid gap-3 sm:grid-cols-3">
            {others.map((p) => (
              <Link
                key={p.slug}
                href={`/products/${p.slug}`}
                className="group rounded-2xl bg-white p-5 shadow-[0_0_0_1px_rgba(0,0,0,0.06)] transition-shadow hover:shadow-[0_0_0_1px_rgba(0,0,0,0.1),0_8px_24px_-12px_rgba(0,0,0,0.15)]"
              >
                <p.icon className="size-5 text-zinc-600" strokeWidth={1.75} />
                <h3 className="mt-4 text-[15px] font-semibold text-zinc-900">{p.title}</h3>
                <p className="mt-1 text-[13.5px] leading-relaxed text-zinc-500">{p.tagline}</p>
                <ArrowRight className="mt-4 size-4 text-zinc-400 transition-transform group-hover:translate-x-0.5 group-hover:text-zinc-900" />
              </Link>
            ))}
          </div>
        </div>
      </section>
    </div>
  );
}
