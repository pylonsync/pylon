// Page-specific styles for `/vs/*` comparison pages. Loaded
// alongside `MARKETING_SHELL_CSS` (which carries the design tokens
// + nav + footer + button system). Keeping these two in separate
// files so neither has to know about the other's internals.

export const COMPARISON_PAGE_CSS = `
.pylon-landing .cmp-hero {
  padding: 64px 0 56px;
  border-bottom: 1px solid var(--line);
  background: linear-gradient(180deg, var(--accent-soft) 0%, transparent 80%);
}
.pylon-landing .cmp-breadcrumb {
  font-size: 13px; color: var(--text-3); margin-bottom: 16px;
}
.pylon-landing .cmp-breadcrumb a { color: var(--text-2); }
.pylon-landing .cmp-breadcrumb a:hover { color: var(--text); }
.pylon-landing .cmp-breadcrumb-sep { padding: 0 8px; color: var(--text-4); }
.pylon-landing .cmp-h1 {
  font-size: clamp(40px, 5vw, 64px); line-height: 1.05;
  letter-spacing: -0.025em; font-weight: 600;
  margin: 0 0 16px; color: var(--ink);
}
.pylon-landing .cmp-h1 .vs { color: var(--accent); }
.pylon-landing .cmp-lede {
  font-size: 18px; line-height: 1.5; color: var(--text-2);
  max-width: 720px; margin: 0 0 28px;
}
.pylon-landing .cmp-ctas { display: flex; gap: 12px; flex-wrap: wrap; }

.pylon-landing .cmp-section {
  padding: 64px 0;
  border-bottom: 1px solid var(--line);
}
.pylon-landing .cmp-section.no-border { border-bottom: none; }
.pylon-landing .cmp-section h2 {
  font-size: clamp(28px, 3vw, 40px); line-height: 1.1;
  letter-spacing: -0.02em; font-weight: 600;
  margin: 0 0 12px;
}
.pylon-landing .cmp-section .cmp-eyebrow {
  font-size: 12px; font-weight: 600; text-transform: uppercase;
  letter-spacing: 0.1em; color: var(--text-3); margin: 0 0 8px;
}
.pylon-landing .cmp-section-lede {
  font-size: 15.5px; color: var(--text-2);
  max-width: 720px; margin: 0 0 32px;
}

/* TL;DR cards */
.pylon-landing .cmp-tldr {
  display: grid; grid-template-columns: 1fr 1fr; gap: 20px;
}
.pylon-landing .cmp-tldr-card {
  background: var(--bg-card); border: 1px solid var(--line);
  border-radius: 14px; padding: 24px;
}
.pylon-landing .cmp-tldr-card.pick-pylon {
  border-color: var(--accent); box-shadow: 0 0 0 4px var(--accent-soft);
}
.pylon-landing .cmp-tldr-card h3 {
  font-size: 14.5px; font-weight: 600; margin: 0 0 12px;
  display: flex; align-items: center; gap: 8px;
}
.pylon-landing .cmp-tldr-card h3 .pill {
  font-size: 10.5px; font-weight: 700; padding: 3px 8px; border-radius: 999px;
  text-transform: uppercase; letter-spacing: 0.08em;
  background: var(--bg-alt); color: var(--text-3);
}
.pylon-landing .cmp-tldr-card.pick-pylon h3 .pill {
  background: var(--accent); color: #fff;
}
.pylon-landing .cmp-tldr-card p {
  font-size: 14.5px; line-height: 1.55; color: var(--text-2); margin: 0;
}

/* Architecture / migration tables */
.pylon-landing .cmp-table-wrap { overflow-x: auto; }
.pylon-landing .cmp-table {
  width: 100%; border-collapse: collapse;
  background: var(--bg-card); border: 1px solid var(--line);
  border-radius: 12px; overflow: hidden;
  font-size: 14px;
}
.pylon-landing .cmp-table th,
.pylon-landing .cmp-table td {
  text-align: left; padding: 12px 16px;
  border-bottom: 1px solid var(--line);
  vertical-align: top;
}
.pylon-landing .cmp-table th {
  font-weight: 600; font-size: 12.5px;
  text-transform: uppercase; letter-spacing: 0.06em;
  color: var(--text-3); background: var(--bg-alt);
}
.pylon-landing .cmp-table th.col-pylon { color: var(--accent); }
.pylon-landing .cmp-table tr:last-child td { border-bottom: none; }
.pylon-landing .cmp-table td.dim {
  font-weight: 500; color: var(--ink); width: 26%;
}
.pylon-landing .cmp-table td.pylon { color: var(--ink); width: 37%; }
.pylon-landing .cmp-table td.competitor { color: var(--text-2); width: 37%; }
.pylon-landing .cmp-table code,
.pylon-landing .cmp-section code {
  background: var(--bg-alt); padding: 1px 6px; border-radius: 4px;
  font-family: var(--font-geist-mono), ui-monospace, monospace;
  font-size: 0.9em;
}

/* Same-shape bullet list */
.pylon-landing .cmp-bullets {
  background: var(--bg-card); border: 1px solid var(--line);
  border-radius: 14px; padding: 24px 28px;
  list-style: none; margin: 0; padding-left: 28px;
}
.pylon-landing .cmp-bullets li {
  padding: 6px 0 6px 24px; position: relative; font-size: 14.5px;
  color: var(--text);
}
.pylon-landing .cmp-bullets li::before {
  content: "✓"; position: absolute; left: 0; top: 6px;
  color: var(--accent); font-weight: 700;
}

/* Item lists (Pylon better / Competitor better) */
.pylon-landing .cmp-items {
  display: grid; gap: 24px; grid-template-columns: 1fr 1fr;
}
.pylon-landing .cmp-item {
  background: var(--bg-card); border: 1px solid var(--line);
  border-radius: 12px; padding: 20px 22px;
}
.pylon-landing .cmp-section.pylon-wins .cmp-item {
  border-color: rgba(139,92,246,.32);
  box-shadow: 0 4px 16px rgba(139,92,246,.06);
}
.pylon-landing .cmp-item h4 {
  font-size: 15.5px; font-weight: 600; margin: 0 0 8px;
  color: var(--ink);
}
.pylon-landing .cmp-item p {
  font-size: 14px; line-height: 1.55; color: var(--text-2); margin: 0;
}

/* Honest-weakness + both-and prose blocks */
.pylon-landing .cmp-prose {
  background: var(--bg-card); border: 1px solid var(--line);
  border-radius: 14px; padding: 28px 32px;
  font-size: 15.5px; line-height: 1.6; color: var(--text);
  max-width: 880px;
}
.pylon-landing .cmp-prose strong { color: var(--ink); }

/* Bottom CTA */
.pylon-landing .cmp-cta {
  background: var(--ink); color: #fff;
  border-radius: 18px; padding: 48px 40px;
  margin: 64px auto 0; max-width: 1216px;
  display: flex; flex-direction: column; gap: 16px; align-items: center;
  text-align: center;
}
.pylon-landing .cmp-cta h2 {
  font-size: clamp(28px, 3vw, 36px); margin: 0;
  font-weight: 600; letter-spacing: -0.02em;
}
.pylon-landing .cmp-cta p {
  font-size: 15.5px; color: rgba(255,255,255,.75); margin: 0;
  max-width: 560px;
}
.pylon-landing .cmp-cta .btn.accent { font-size: 15px; padding: 12px 22px; }

/* /vs index page */
.pylon-landing .vs-index {
  padding: 64px 0 24px;
}
.pylon-landing .vs-index-grid {
  display: grid; grid-template-columns: repeat(2, 1fr); gap: 20px;
  margin-top: 32px;
}
.pylon-landing .vs-card {
  display: block; background: var(--bg-card);
  border: 1px solid var(--line); border-radius: 14px;
  padding: 24px 28px; transition: border-color .15s ease, box-shadow .15s ease;
}
.pylon-landing .vs-card:hover {
  border-color: var(--accent); box-shadow: 0 4px 16px rgba(139,92,246,.10);
}
.pylon-landing .vs-card h3 {
  font-size: 18px; font-weight: 600; margin: 0 0 8px; color: var(--ink);
}
.pylon-landing .vs-card p {
  font-size: 14.5px; color: var(--text-2); margin: 0;
}
.pylon-landing .vs-card .arrow {
  display: inline-block; margin-top: 12px; color: var(--accent);
  font-size: 13.5px; font-weight: 500;
}

@media (max-width: 880px) {
  .pylon-landing .cmp-tldr,
  .pylon-landing .cmp-items,
  .pylon-landing .vs-index-grid { grid-template-columns: 1fr; }
}
`;
