import React from "react";
import { type Metadata } from "@pylonsync/react";
import { WRAP, PROSE, Divider, SectionTitle, ImagePlaceholder } from "@/components/marketing";
import { NewsletterSignup } from "./newsletter-signup";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/page.tsx` → `/`. A server-rendered personal site set in one narrow
// column. Hero, about, offerings, writing, testimonials, and the closing
// section are static server HTML (SEO + first paint); the newsletter form in
// the hero is a client island with a live subscriber counter. All copy comes
// from siteConfig. Doesn't read `auth`, so the public page stays cacheable.
export default function LandingPage() {
  const { hero, about, offerings, writing, testimonials, newsletter, links } = siteConfig;

  return (
    <>
      {/* HERO */}
      <section className={`${WRAP} pt-14 pb-12 sm:pt-20`}>
        <div className="flex items-center gap-5">
          <div className="w-20 shrink-0 sm:w-24">
            <ImagePlaceholder shape="circle" title={hero.name} src="/images/headshot.jpg" />
          </div>
          <div>
            <h1 className="text-[2rem] font-medium leading-[1.1] text-ink sm:text-[2.5rem]">
              {hero.name}
            </h1>
            <p className="mt-1.5 text-[17.5px] italic leading-snug text-ink-2">{hero.tagline}</p>
          </div>
        </div>
        <p className={`mt-8 ${PROSE}`}>{hero.intro}</p>
        <div id="newsletter" className="mt-8 scroll-mt-20">
          <p className={PROSE}>
            <em>{newsletter.name}</em> is my newsletter. {newsletter.subcopy}
          </p>
          <div className="mt-4">
            <NewsletterSignup newsletter={newsletter} />
          </div>
        </div>
      </section>

      {/* ABOUT */}
      <Divider />
      <section className={`${WRAP} py-12`}>
        <SectionTitle>{about.headline}</SectionTitle>
        <div className={`mt-5 space-y-5 ${PROSE}`}>
          {about.paragraphs.map((p) => (
            <p key={p}>{p}</p>
          ))}
        </div>
      </section>

      {/* WHAT I DO */}
      <Divider />
      <section className={`${WRAP} py-12`}>
        <SectionTitle>{offerings.headline}</SectionTitle>
        <dl className="mt-5 divide-y divide-rule">
          {offerings.items.map((o) => (
            <div key={o.title} className="py-5 first:pt-0 last:pb-0">
              <dt className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-1">
                <span className="text-[18px] font-medium text-ink">{o.title}</span>
                {o.price ? <span className="text-[16px] italic text-brand">{o.price}</span> : null}
              </dt>
              <dd className={`mt-1.5 ${PROSE}`}>{o.body}</dd>
            </div>
          ))}
        </dl>
      </section>

      {/* RECENT WRITING */}
      {writing && writing.items.length > 0 ? (
        <>
          <Divider />
          <section className={`${WRAP} py-12`}>
            <SectionTitle>{writing.headline}</SectionTitle>
            <ul className="mt-5 divide-y divide-rule">
              {writing.items.map((issue) => (
                <li key={issue.title} className="py-4 first:pt-0 last:pb-0">
                  <a href={issue.href} className="group block">
                    <div className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-1">
                      <span className="text-[18px] font-medium text-ink underline-offset-4 group-hover:underline">
                        {issue.title}
                      </span>
                      <time className="text-[15px] text-ink-2">{issue.date}</time>
                    </div>
                    {issue.summary ? <p className={`mt-1 ${PROSE}`}>{issue.summary}</p> : null}
                  </a>
                </li>
              ))}
            </ul>
          </section>
        </>
      ) : null}

      {/* TESTIMONIALS */}
      {testimonials && testimonials.items.length > 0 ? (
        <>
          <Divider />
          <section className={`${WRAP} py-12`}>
            <SectionTitle>{testimonials.headline}</SectionTitle>
            <div className="mt-5 space-y-7">
              {testimonials.items.map((t) => (
                <figure key={t.name} className="border-l-2 border-brand pl-5">
                  <blockquote className={`italic ${PROSE}`}>&ldquo;{t.quote}&rdquo;</blockquote>
                  <figcaption className="mt-2 text-[15px] text-ink-2">
                    {t.name}, {t.role}
                  </figcaption>
                </figure>
              ))}
            </div>
          </section>
        </>
      ) : null}

      {/* WORK WITH ME */}
      {links ? (
        <>
          <Divider />
          <section id="work" className={`${WRAP} py-12 scroll-mt-20`}>
            <SectionTitle>{links.headline}</SectionTitle>
            <p className={`mt-5 ${PROSE}`}>{links.body}</p>
            {links.items.length > 0 ? (
              <ul className={`mt-5 space-y-2 ${PROSE}`}>
                {links.items.map((l) => (
                  <li key={l.label}>
                    <a href={l.href} className="font-medium text-ink underline decoration-brand/50 underline-offset-4 hover:decoration-brand">
                      {l.label}
                    </a>
                    {l.note ? <span className="text-ink-2"> · {l.note}</span> : null}
                  </li>
                ))}
              </ul>
            ) : null}
          </section>
        </>
      ) : null}
    </>
  );
}
