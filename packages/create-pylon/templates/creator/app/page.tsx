import React from "react";
import { type Metadata } from "@pylonsync/react";
import {
  WRAP,
  Eyebrow,
  Divider,
  SectionHead,
  initials,
  ImagePlaceholder,
} from "@/components/marketing";
import { NewsletterSignup } from "./newsletter-signup";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/page.tsx` → `/`. A server-rendered personal-brand site. Hero, about,
// offerings, testimonials, and links are static server HTML (SEO + first paint);
// the newsletter section (#newsletter) is a client island with a live
// subscriber counter. All copy comes from siteConfig. Doesn't read `auth`, so
// the public page stays cacheable.
export default function LandingPage() {
  const { hero, about, offerings, testimonials, newsletter, links } = siteConfig;

  return (
    <div className="bg-white text-zinc-900">
      {/* HERO */}
      <section className={`${WRAP} pt-16 pb-14 sm:pt-20`}>
        <div className="grid items-center gap-10 sm:grid-cols-[1fr_auto]">
          <div>
            <span className="inline-flex items-center gap-2 rounded-full border border-zinc-200 bg-white py-1 pl-1.5 pr-3 text-[13px] text-zinc-600 shadow-sm">
              <span className="inline-block size-1.5 rounded-full bg-brand" />
              {hero.badge}
            </span>
            <h1 className="mt-5 text-balance text-[2.5rem] font-semibold leading-[1.04] tracking-[-0.02em] sm:text-[3rem]">
              {hero.name}
            </h1>
            <p className="mt-2 text-[18px] font-medium text-brand">{hero.tagline}</p>
            <p className="mt-5 max-w-xl text-[16px] leading-relaxed text-zinc-500">{hero.intro}</p>
            <div className="mt-7 flex flex-wrap items-center gap-4">
              <a href="#newsletter" className="inline-flex items-center rounded-full bg-zinc-900 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-zinc-700">
                Read the newsletter
              </a>
              <a href="#work" className="text-sm font-medium text-zinc-700 hover:text-zinc-900">
                Work with me →
              </a>
            </div>
          </div>
          {/* Profile photo — replace the placeholder with a real headshot. */}
          <div className="order-first w-32 sm:order-none sm:w-44">
            <ImagePlaceholder
              shape="square"
              title="Your headshot"
              hint="Replace in app/page.tsx"
            />
          </div>
        </div>
      </section>

      {/* ABOUT */}
      <Divider />
      <section className={`${WRAP} py-14`}>
        <div className="grid gap-8 lg:grid-cols-[0.8fr_1.2fr]">
          <SectionHead eyebrow={about.eyebrow} title={about.headline} />
          <div className="space-y-4 text-[15px] leading-relaxed text-zinc-600">
            {about.paragraphs.map((p) => (
              <p key={p}>{p}</p>
            ))}
          </div>
        </div>
      </section>

      {/* OFFERINGS */}
      <Divider />
      <section id="work" className={`${WRAP} py-14`}>
        <SectionHead eyebrow={offerings.eyebrow} title={offerings.headline} />
        <div className="mt-8 grid gap-5 sm:grid-cols-3">
          {offerings.items.map((o) => (
            <div key={o.title} className="flex flex-col rounded-2xl border border-zinc-200 bg-paper p-6">
              <h3 className="text-[15px] font-semibold text-zinc-900">{o.title}</h3>
              <p className="mt-2 flex-1 text-[14px] leading-relaxed text-zinc-500">{o.body}</p>
              {o.price ? (
                <p className="mt-4 text-[13px] font-medium uppercase tracking-wide text-brand">{o.price}</p>
              ) : null}
            </div>
          ))}
        </div>
      </section>

      {/* TESTIMONIALS */}
      {testimonials && testimonials.items.length > 0 ? (
        <>
          <Divider />
          <section className={`${WRAP} py-14`}>
            <SectionHead eyebrow={testimonials.eyebrow} title={testimonials.headline} />
            <div className="mt-8 grid gap-5 sm:grid-cols-3">
              {testimonials.items.map((t) => (
                <figure key={t.name} className="flex flex-col rounded-2xl border border-zinc-200 bg-paper p-6">
                  <blockquote className="text-[14px] leading-relaxed text-zinc-600">&ldquo;{t.quote}&rdquo;</blockquote>
                  <figcaption className="mt-5 flex items-center gap-3">
                    <span className="flex size-8 items-center justify-center rounded-full bg-zinc-200 text-[11px] font-semibold text-zinc-500">
                      {initials(t.name)}
                    </span>
                    <div className="leading-tight">
                      <div className="text-[13px] font-semibold">{t.name}</div>
                      <div className="text-[12px] text-zinc-500">{t.role}</div>
                    </div>
                  </figcaption>
                </figure>
              ))}
            </div>
          </section>
        </>
      ) : null}

      {/* NEWSLETTER (live) */}
      <Divider />
      <section id="newsletter" className={`${WRAP} py-14`}>
        <div className="rounded-2xl border border-zinc-200 bg-paper p-7 sm:p-10">
          <Eyebrow>{newsletter.eyebrow}</Eyebrow>
          <h2 className="mt-4 max-w-2xl text-balance text-2xl font-semibold leading-[1.15] tracking-[-0.02em] sm:text-3xl">
            {newsletter.headline}
          </h2>
          <p className="mt-4 max-w-xl text-[15px] leading-relaxed text-zinc-500">{newsletter.subcopy}</p>
          <div className="mt-7">
            <NewsletterSignup newsletter={newsletter} />
          </div>
        </div>
      </section>

      {/* LINKS */}
      {links && links.items.length > 0 ? (
        <>
          <Divider />
          <section className={`${WRAP} py-14`}>
            <SectionHead eyebrow={links.eyebrow} title={links.headline} />
            <div className="mt-8 grid gap-3 sm:grid-cols-3">
              {links.items.map((l) => (
                <a
                  key={l.label}
                  href={l.href}
                  className="group rounded-xl border border-zinc-200 bg-white px-5 py-4 transition-colors hover:border-zinc-300"
                >
                  <div className="flex items-center justify-between text-[14px] font-medium text-zinc-900">
                    {l.label}
                    <span className="text-zinc-300 transition-colors group-hover:text-brand">→</span>
                  </div>
                  {l.note ? <p className="mt-1 text-[12.5px] text-zinc-500">{l.note}</p> : null}
                </a>
              ))}
            </div>
          </section>
        </>
      ) : null}
    </div>
  );
}
