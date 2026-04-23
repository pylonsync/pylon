/**
 * Section renderers — each a rich, complete designed layout the user
 * can drop onto a page as a single block. Keeps new sections decoupled
 * from StageApp.tsx; just import and register.
 */

import React from "react";
import { Icon } from "./icons";

type Props = Record<string, any>;

// ---------------------------------------------------------------------------
// Hero — centered
// ---------------------------------------------------------------------------

export function HeroCentered({ props }: { props: Props }) {
  return (
    <section className="sec hero-centered">
      {props.eyebrow && <span className="sec-eyebrow">{props.eyebrow}</span>}
      <h1 className="sec-h1">{props.title || "Your bold headline."}</h1>
      {props.subtitle && <p className="sec-sub">{props.subtitle}</p>}
      <div className="sec-cta-row">
        {props.primaryCta?.text && (
          <a className="sec-btn primary" href={props.primaryCta.href || "#"} onClick={(e) => e.preventDefault()}>
            {props.primaryCta.text} <Icon name="ArrowRight" size={14} />
          </a>
        )}
        {props.secondaryCta?.text && (
          <a className="sec-btn ghost" href={props.secondaryCta.href || "#"} onClick={(e) => e.preventDefault()}>
            {props.secondaryCta.text}
          </a>
        )}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Hero — split with image
// ---------------------------------------------------------------------------

export function HeroSplit({ props }: { props: Props }) {
  return (
    <section className="sec hero-split">
      <div className="hero-split-text">
        {props.eyebrow && <span className="sec-eyebrow">{props.eyebrow}</span>}
        <h1 className="sec-h1">{props.title || "Your bold headline."}</h1>
        {props.subtitle && <p className="sec-sub">{props.subtitle}</p>}
        {props.primaryCta?.text && (
          <a className="sec-btn primary" href={props.primaryCta.href || "#"} onClick={(e) => e.preventDefault()}>
            {props.primaryCta.text} <Icon name="ArrowRight" size={14} />
          </a>
        )}
      </div>
      <div className="hero-split-image">
        <img src={props.image || "https://images.unsplash.com/photo-1498050108023-c5249f4df085?w=1200&h=900&fit=crop"} alt="" />
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Feature grid
// ---------------------------------------------------------------------------

export function FeatureGrid({ props }: { props: Props }) {
  const items = Array.isArray(props.items) ? props.items : [];
  return (
    <section className="sec feature-grid">
      {props.eyebrow && <span className="sec-eyebrow">{props.eyebrow}</span>}
      {props.title && <h2 className="sec-h2">{props.title}</h2>}
      <div className="feature-grid-items">
        {items.map((it: any, i: number) => (
          <div key={i} className="feature-card">
            <div className="feature-icon">
              <Icon name={it.icon || "Sparkle"} size={22} strokeWidth={2} />
            </div>
            <h3 className="feature-title">{it.title || "Feature"}</h3>
            <p className="feature-text">{it.text || ""}</p>
          </div>
        ))}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

export function Stats({ props }: { props: Props }) {
  const items = Array.isArray(props.items) ? props.items : [];
  return (
    <section className="sec stats">
      <div className="stats-row">
        {items.map((it: any, i: number) => (
          <div key={i} className="stat-cell">
            <div className="stat-value">{it.value}</div>
            <div className="stat-label">{it.label}</div>
          </div>
        ))}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Logo cloud
// ---------------------------------------------------------------------------

export function LogoCloud({ props }: { props: Props }) {
  const logos = Array.isArray(props.logos) ? props.logos : [];
  return (
    <section className="sec logo-cloud">
      <div className="logo-cloud-title">{props.title || "Trusted by"}</div>
      <div className="logo-cloud-row">
        {logos.map((name: string, i: number) => (
          <div key={i} className="logo-mark">{name}</div>
        ))}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Testimonial
// ---------------------------------------------------------------------------

export function Testimonial({ props }: { props: Props }) {
  return (
    <section className="sec testimonial">
      <div className="quote-mark"><Icon name="Quote" size={36} strokeWidth={2} /></div>
      <blockquote className="quote-body">{props.quote || ""}</blockquote>
      <div className="quote-byline">
        {props.avatar && <img className="quote-avatar" src={props.avatar} alt="" />}
        <div>
          <div className="quote-author">{props.author || ""}</div>
          <div className="quote-role">{props.role || ""}</div>
        </div>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Pricing tiers
// ---------------------------------------------------------------------------

export function Pricing({ props }: { props: Props }) {
  const tiers = Array.isArray(props.tiers) ? props.tiers : [];
  return (
    <section className="sec pricing">
      {props.eyebrow && <span className="sec-eyebrow">{props.eyebrow}</span>}
      {props.title && <h2 className="sec-h2">{props.title}</h2>}
      <div className="pricing-row">
        {tiers.map((t: any, i: number) => (
          <div key={i} className={`pricing-card ${t.highlight ? "highlight" : ""}`}>
            {t.highlight && <div className="pricing-ribbon">Most popular</div>}
            <div className="pricing-name">{t.name}</div>
            <div className="pricing-price">
              <span className="price-amt">{t.price}</span>
              <span className="price-per">{t.period}</span>
            </div>
            <ul className="pricing-features">
              {(t.features || []).map((f: string, j: number) => (
                <li key={j}>
                  <span className="check"><Icon name="Check" size={14} strokeWidth={2.5} /></span> {f}
                </li>
              ))}
            </ul>
            <a className={`sec-btn ${t.highlight ? "primary" : "outline"}`} href="#" onClick={(e) => e.preventDefault()}>
              {t.cta || "Get started"}
            </a>
          </div>
        ))}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// CTA banner
// ---------------------------------------------------------------------------

export function CtaBanner({ props }: { props: Props }) {
  return (
    <section className="sec cta-banner">
      <div className="cta-inner">
        <h2 className="sec-h2 light">{props.title || "Ready to get started?"}</h2>
        {props.subtitle && <p className="sec-sub light">{props.subtitle}</p>}
        {props.primaryCta?.text && (
          <a className="sec-btn on-dark" href={props.primaryCta.href || "#"} onClick={(e) => e.preventDefault()}>
            {props.primaryCta.text}
          </a>
        )}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// FAQ — stacked (not actually collapsible; for demo always expanded)
// ---------------------------------------------------------------------------

export function FAQ({ props }: { props: Props }) {
  const items = Array.isArray(props.items) ? props.items : [];
  const [openIdx, setOpenIdx] = React.useState<number>(0);
  return (
    <section className="sec faq">
      {props.title && <h2 className="sec-h2">{props.title}</h2>}
      <div className="faq-list">
        {items.map((it: any, i: number) => (
          <div key={i} className={`faq-item ${openIdx === i ? "open" : ""}`}>
            <button className="faq-q" onClick={() => setOpenIdx(openIdx === i ? -1 : i)}>
              <span>{it.q}</span>
              <span className="faq-chev">
                <Icon name={openIdx === i ? "Minus" : "Plus"} size={16} strokeWidth={2} />
              </span>
            </button>
            {openIdx === i && <div className="faq-a">{it.a}</div>}
          </div>
        ))}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Footer
// ---------------------------------------------------------------------------

export function Footer({ props }: { props: Props }) {
  const columns = Array.isArray(props.columns) ? props.columns : [];
  return (
    <section className="sec footer">
      <div className="footer-top">
        <div className="footer-brand">{props.tagline || "Ship fast."}</div>
        <div className="footer-cols">
          {columns.map((c: any, i: number) => (
            <div key={i} className="footer-col">
              <div className="footer-col-title">{c.title}</div>
              {(c.links || []).map((l: any, j: number) => (
                <a key={j} href={l.href} onClick={(e) => e.preventDefault()}>{l.text}</a>
              ))}
            </div>
          ))}
        </div>
      </div>
      <div className="footer-bottom">{props.copyright}</div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

export function renderSection(type: string, props: Props): React.ReactNode | null {
  switch (type) {
    case "hero-centered": return <HeroCentered props={props} />;
    case "hero-split": return <HeroSplit props={props} />;
    case "feature-grid": return <FeatureGrid props={props} />;
    case "stats": return <Stats props={props} />;
    case "logo-cloud": return <LogoCloud props={props} />;
    case "testimonial": return <Testimonial props={props} />;
    case "pricing": return <Pricing props={props} />;
    case "cta-banner": return <CtaBanner props={props} />;
    case "faq": return <FAQ props={props} />;
    case "footer": return <Footer props={props} />;
    default: return null;
  }
}

export const SECTION_TYPES = new Set([
  "hero-centered", "hero-split", "feature-grid", "stats", "logo-cloud",
  "testimonial", "pricing", "cta-banner", "faq", "footer",
]);
