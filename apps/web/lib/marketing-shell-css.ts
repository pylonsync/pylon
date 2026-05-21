// Marketing chrome styles — design tokens, nav, footer, button
// system, shared by every marketing page that uses
// `<MarketingShell>` (i.e., everything except the giant `/` landing
// page, which carries its own DESIGN_CSS inline).
//
// Scoped to `.pylon-landing` so wrapping any page in that class
// activates the design system without polluting the rest of the
// site (the cloud dashboard, for example, uses different tokens).
//
// Stays in sync with the landing page's DESIGN_CSS by hand. Future
// split: lift the shared bits into a single source and import here
// + on the landing page.

export const MARKETING_SHELL_CSS = `
.pylon-landing {
  --bg: #fafaf7;
  --bg-alt: #f3f3ee;
  --bg-card: #ffffff;
  --ink: #0a0a0b;
  --ink-2: #1a1a1d;
  --ink-3: #3a3a40;
  --text: var(--ink);
  --text-2: #4a4a50;
  --text-3: #7a7a80;
  --text-4: #aaaab0;
  --line: rgba(0,0,0,.08);
  --line-2: rgba(0,0,0,.16);
  --accent: #8B5CF6;
  --accent-soft: #f5f3ff;
  --accent-deep: #7C3AED;
  --shadow-sm: 0 1px 2px rgba(0,0,0,.06);
  --shadow-md: 0 4px 16px rgba(0,0,0,.08);

  background: var(--bg);
  color: var(--ink);
  font-family: var(--font-geist-sans), system-ui, -apple-system, sans-serif;
  font-size: 16px;
  line-height: 1.6;
  font-feature-settings: "ss01", "cv11";
  min-height: 100vh;
}
.pylon-landing * { box-sizing: border-box; }
.pylon-landing a { color: inherit; text-decoration: none; }

.pylon-landing .shell { max-width: 1280px; margin: 0 auto; padding: 0 32px; }
.pylon-landing .shell-narrow { max-width: 880px; margin: 0 auto; padding: 0 32px; }

/* === NAV === */
.pylon-landing .nav {
  position: sticky; top: 0; z-index: 50;
  background: rgba(250,250,247,.85);
  backdrop-filter: saturate(140%) blur(10px);
  -webkit-backdrop-filter: saturate(140%) blur(10px);
  border-bottom: 1px solid var(--line);
}
.pylon-landing .nav-inner {
  display: flex; align-items: center; justify-content: space-between;
  padding: 14px 0;
}
.pylon-landing .nav .brand {
  display: inline-flex; align-items: center; gap: 8px;
  font-weight: 600; font-size: 16px; color: var(--ink);
}
.pylon-landing .nav-links {
  display: flex; gap: 4px; list-style: none; padding: 0; margin: 0;
  font-size: 13.5px;
}
.pylon-landing .nav-links li a {
  display: inline-block; padding: 6px 12px; border-radius: 8px;
  color: var(--text-2);
  transition: color .15s ease, background .15s ease;
}
.pylon-landing .nav-links li a:hover { color: var(--text); background: var(--bg-alt); }
.pylon-landing .nav-cta { display: flex; gap: 8px; align-items: center; }
.pylon-landing .nav-github {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 6px 10px; border-radius: 8px;
  color: var(--text-2); font-size: 12.5px; font-weight: 500;
  transition: color .15s ease, background .15s ease;
}
.pylon-landing .nav-github:hover { color: var(--text); background: var(--bg-alt); }
.pylon-landing .nav-github-stars { font-variant-numeric: tabular-nums; }

/* === BUTTONS === */
.pylon-landing .btn {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 9px 16px; border-radius: 10px; font-size: 13.5px; font-weight: 500;
  border: 1px solid transparent; cursor: pointer; line-height: 1;
  transition: background .15s ease, border-color .15s ease, color .15s ease, box-shadow .15s ease;
}
.pylon-landing .btn.ghost { color: var(--text-2); }
.pylon-landing .btn.ghost:hover { color: var(--text); background: var(--bg-alt); }
.pylon-landing .btn.line { border-color: var(--line-2); color: var(--text); background: var(--bg-card); }
.pylon-landing .btn.line:hover { border-color: var(--ink); box-shadow: var(--shadow-sm); }
.pylon-landing .btn.dark { background: var(--ink); color: #fff; border-color: var(--ink); }
.pylon-landing .btn.dark:hover { background: #000; box-shadow: 0 4px 14px rgba(0,0,0,.20); }
.pylon-landing .btn.accent { background: var(--accent); color: #fff; border-color: var(--accent); }
.pylon-landing .btn.accent:hover { background: var(--accent-deep); border-color: var(--accent-deep); box-shadow: 0 4px 14px rgba(139,92,246,.28); }

/* === FOOTER === */
.pylon-landing .footer {
  border-top: 1px solid var(--line);
  margin-top: 80px;
  padding: 56px 0 40px;
  background: var(--bg-alt);
}
.pylon-landing .footer-inner {
  display: flex; flex-direction: column; gap: 36px;
}
.pylon-landing .footer-brand {
  display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
  font-weight: 600;
}
.pylon-landing .footer-tag {
  font-weight: 400; color: var(--text-3); font-size: 13.5px;
  border-left: 1px solid var(--line); padding-left: 12px; margin-left: 4px;
}
.pylon-landing .footer-cols {
  display: grid; grid-template-columns: repeat(4, 1fr); gap: 32px;
}
.pylon-landing .footer-col { display: flex; flex-direction: column; gap: 8px; }
.pylon-landing .footer-col h6 {
  font-size: 11.5px; font-weight: 600; text-transform: uppercase;
  letter-spacing: 0.08em; color: var(--text-3); margin: 0 0 4px;
}
.pylon-landing .footer-col a {
  font-size: 13.5px; color: var(--text-2);
  transition: color .15s ease;
}
.pylon-landing .footer-col a:hover { color: var(--text); }
.pylon-landing .footer-base {
  display: flex; justify-content: space-between; gap: 16px; flex-wrap: wrap;
  padding-top: 24px; border-top: 1px solid var(--line);
  font-size: 12.5px; color: var(--text-3);
}

@media (max-width: 768px) {
  .pylon-landing .nav-links { display: none; }
  .pylon-landing .footer-cols { grid-template-columns: repeat(2, 1fr); }
  .pylon-landing .nav-cta .btn.ghost { display: none; }
}
@media (max-width: 480px) {
  .pylon-landing .footer-cols { grid-template-columns: 1fr; }
}
`;
