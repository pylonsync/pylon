"use client";

import Link from "next/link";
import { useState } from "react";
import { PylonMark } from "@/components/pylon-logo";

const CLOUD_CSS = `
html, body { background: #fafaf9; color: #18181b; }

.pylon-cloud {
  --bg: #fafaf9;
  --bg-alt: #f4f3f0;
  --bg-card: #ffffff;
  --ink: #0a0a0b;
  --text: #18181b;
  --text-2: #52525b;
  --text-3: #a1a1aa;
  --line: #e7e5e2;
  --line-2: #d4d4d0;
  --accent: #ff5b1f;
  --accent-soft: #fff1ea;
  --accent-deep: #c9420f;
  --pos: #16a34a;
  --pos-soft: #e7f6ec;
  --code-bg: #0c0c0f;
  --code-text: #ededee;
  --code-mute: #71717a;
  --code-green: #98e08c;
  --code-orange: #ffb86b;
  --shadow-sm: 0 1px 2px rgba(15,15,20,.04), 0 1px 1px rgba(15,15,20,.02);
  --shadow-md: 0 8px 24px -8px rgba(15,15,20,.10), 0 2px 6px rgba(15,15,20,.04);
  --shadow-lg: 0 24px 48px -16px rgba(15,15,20,.18), 0 4px 12px rgba(15,15,20,.06);
  background: var(--bg);
  color: var(--text);
  font-family: "Geist", -apple-system, system-ui, sans-serif;
  font-feature-settings: "ss01","cv11";
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  min-height: 100vh;
  overflow-x: hidden;
  -webkit-tap-highlight-color: rgba(255, 91, 31, .18);
}
.pylon-cloud * { box-sizing: border-box; }
.pylon-cloud .mono { font-family: "Geist Mono", ui-monospace, monospace; }
.pylon-cloud .serif { font-family: "Instrument Serif", "Times New Roman", serif; font-style: italic; }
.pylon-cloud a { color: inherit; text-decoration: none; }
.pylon-cloud button { font-family: inherit; cursor: pointer; }
.pylon-cloud a, .pylon-cloud button { touch-action: manipulation; }
.pylon-cloud a:focus-visible, .pylon-cloud button:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 3px;
}

.pylon-cloud .shell { max-width: 1200px; margin: 0 auto; padding: 0 32px; }

/* === NAV === */
.pylon-cloud .nav {
  position: sticky; top: 0; z-index: 50;
  background: rgba(250, 250, 249, .82);
  backdrop-filter: blur(14px) saturate(140%);
  -webkit-backdrop-filter: blur(14px) saturate(140%);
  border-bottom: 1px solid var(--line);
}
.pylon-cloud .nav-inner { display: flex; align-items: center; justify-content: space-between; padding: 14px 0; gap: 12px; }
.pylon-cloud .brand { display: flex; align-items: center; gap: 9px; font-weight: 600; letter-spacing: -.02em; font-size: 16px; }
.pylon-cloud .brand .tag {
  font-family: "Geist Mono", monospace; font-size: 10.5px;
  color: var(--accent); background: var(--accent-soft);
  padding: 2px 7px; border-radius: 4px; letter-spacing: .06em;
  margin-left: 4px; font-weight: 500;
}
.pylon-cloud .nav-links { display: flex; gap: 4px; list-style: none; padding: 0; margin: 0; font-size: 13.5px; }
.pylon-cloud .nav-links li a { display: inline-block; padding: 6px 12px; border-radius: 8px; color: var(--text-2); transition: color .15s ease, background .15s ease; }
.pylon-cloud .nav-links li a:hover { color: var(--text); background: var(--bg-alt); }
.pylon-cloud .nav-cta { display: flex; gap: 8px; align-items: center; min-width: 0; }
.pylon-cloud .btn {
  display: inline-flex; align-items: center; gap: 8px;
  padding: 8px 14px; border-radius: 8px;
  font-size: 13.5px; font-weight: 500;
  border: 1px solid transparent;
  transition: background .15s ease, border-color .15s ease, color .15s ease, box-shadow .15s ease;
  text-decoration: none;
  background: transparent; color: inherit;
  white-space: nowrap;
}
.pylon-cloud .btn.ghost { color: var(--text-2); }
.pylon-cloud .btn.ghost:hover { color: var(--text); background: var(--bg-alt); }
.pylon-cloud .btn.line { border-color: var(--line-2); color: var(--text); background: var(--bg-card); }
.pylon-cloud .btn.line:hover { border-color: var(--ink); box-shadow: var(--shadow-sm); }
.pylon-cloud .btn.dark { background: var(--ink); color: #fff; border-color: var(--ink); }
.pylon-cloud .btn.dark:hover { background: #000; box-shadow: 0 4px 14px rgba(0,0,0,.20); }
.pylon-cloud .btn.accent { background: var(--accent); color: #fff; border-color: var(--accent); }
.pylon-cloud .btn.accent:hover { background: var(--accent-deep); border-color: var(--accent-deep); box-shadow: 0 4px 14px rgba(255,91,31,.28); }

/* === HERO === */
.pylon-cloud .hero { padding: 88px 0 0; position: relative; overflow: hidden; }
.pylon-cloud .hero-grid {
  position: absolute; inset: 0;
  background-image:
    linear-gradient(var(--line) 1px, transparent 1px),
    linear-gradient(90deg, var(--line) 1px, transparent 1px);
  background-size: 56px 56px;
  mask-image: radial-gradient(700px 400px at 50% 30%, #000 30%, transparent 80%);
  -webkit-mask-image: radial-gradient(700px 400px at 50% 30%, #000 30%, transparent 80%);
  opacity: .55;
  pointer-events: none;
}
.pylon-cloud .hero-tag {
  display: inline-flex; align-items: center; gap: 8px;
  padding: 5px 12px 5px 6px;
  background: var(--bg-card);
  border: 1px solid var(--line-2);
  border-radius: 999px;
  font-size: 12.5px; color: var(--text-2);
  box-shadow: var(--shadow-sm);
  position: relative; z-index: 1;
}
.pylon-cloud .hero-tag .pill {
  background: var(--accent-soft); color: var(--accent-deep);
  padding: 2px 9px; border-radius: 999px; font-size: 11px; font-weight: 600;
  letter-spacing: .02em;
}
.pylon-cloud .hero-text { position: relative; max-width: 880px; }
.pylon-cloud h1.h1 {
  font-size: clamp(44px, 6.2vw, 84px);
  line-height: 1;
  letter-spacing: -.045em;
  font-weight: 600;
  margin: 24px 0 22px;
  color: var(--ink);
  max-width: 14ch;
}
.pylon-cloud h1.h1 .serif { color: var(--text-2); font-weight: 400; letter-spacing: -.02em; }
.pylon-cloud .hero p.lede {
  font-size: 19.5px; line-height: 1.45;
  max-width: 580px; color: var(--text-2);
  margin: 0 0 32px;
}
.pylon-cloud .hero p.lede b { color: var(--ink); font-weight: 500; }
.pylon-cloud .hero-ctas { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
.pylon-cloud .term-pill {
  display: inline-flex; align-items: center; gap: 10px;
  background: var(--bg-card); border: 1px solid var(--line-2);
  padding: 7px 12px 7px 14px; border-radius: 8px;
  font-family: "Geist Mono", monospace; font-size: 13px;
  color: var(--text); box-shadow: var(--shadow-sm);
  max-width: 100%;
  min-width: 0;
}
.pylon-cloud .term-pill .prompt { color: var(--text-3); flex-shrink: 0; }
.pylon-cloud .term-pill .cmd-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pylon-cloud .term-pill .copy {
  display: inline-flex; align-items: center; justify-content: center;
  width: 24px; height: 24px; border-radius: 5px; color: var(--text-3);
  background: var(--bg-alt); flex: 0 0 24px;
}

.pylon-cloud .hero-meta {
  margin-top: 56px; padding: 18px 0;
  border-top: 1px solid var(--line);
  border-bottom: 1px solid var(--line);
  display: grid; grid-template-columns: repeat(4, 1fr); gap: 24px;
}
.pylon-cloud .hero-meta .item .k { font-family: "Geist Mono", monospace; font-size: 11px; color: var(--text-3); text-transform: uppercase; letter-spacing: .08em; }
.pylon-cloud .hero-meta .item .v { font-size: 16px; font-weight: 500; letter-spacing: -.01em; margin-top: 4px; color: var(--ink); }
.pylon-cloud .hero-meta .item .v .serif { color: var(--accent); font-size: 18px; }

/* === DEPLOY DEMO === */
.pylon-cloud .deploy-frame {
  margin: 56px 0 0; position: relative;
  border-radius: 14px;
  background: var(--code-bg);
  border: 1px solid #1c1c22;
  box-shadow: var(--shadow-lg);
  overflow: hidden;
  font-family: "Geist Mono", monospace;
}
.pylon-cloud .deploy-head {
  display: flex; align-items: center; gap: 10px;
  padding: 12px 16px; border-bottom: 1px solid #1c1c22;
  font-size: 11px; color: var(--code-mute); letter-spacing: .06em; text-transform: uppercase;
  background: linear-gradient(180deg, #16161b, #0c0c10);
}
.pylon-cloud .deploy-head .dots { display: flex; gap: 6px; }
.pylon-cloud .deploy-head .dots i { width: 11px; height: 11px; border-radius: 50%; background: #2a2a32; display: inline-block; }
.pylon-cloud .deploy-body {
  padding: 22px 22px 26px; line-height: 1.75; font-size: 13px;
  color: var(--code-text);
}
.pylon-cloud .deploy-body .pr { color: var(--code-orange); }
.pylon-cloud .deploy-body .ok { color: var(--code-green); }
.pylon-cloud .deploy-body .dim { color: var(--code-mute); }
.pylon-cloud .deploy-body .url { color: #79b8ff; }
.pylon-cloud .deploy-body .blink { display: inline-block; width: 7px; height: 14px; background: var(--code-text); vertical-align: middle; margin-left: 2px; animation: pylon-cloud-blink 1s step-end infinite; }
@keyframes pylon-cloud-blink { 50% { opacity: 0; } }

/* === SECTIONS === */
.pylon-cloud section.block { padding: 112px 0; position: relative; border-bottom: 1px solid var(--line); }
.pylon-cloud .eyebrow {
  display: inline-flex; align-items: center; gap: 8px;
  font-family: "Geist Mono", monospace; font-size: 11.5px;
  color: var(--accent); text-transform: uppercase; letter-spacing: .14em; font-weight: 500;
}
.pylon-cloud .eyebrow::before { content: ""; width: 14px; height: 1px; background: var(--accent); }
.pylon-cloud h2.h2 {
  font-size: clamp(36px, 4.2vw, 56px);
  letter-spacing: -.035em; line-height: 1.04;
  font-weight: 600; color: var(--ink);
  margin: 16px 0 18px; max-width: 22ch;
}
.pylon-cloud h2.h2 .serif { color: var(--text-2); font-weight: 400; }
.pylon-cloud .lede { font-size: 18px; color: var(--text-2); line-height: 1.5; max-width: 620px; }

/* === VALUE TILES === */
.pylon-cloud .tiles {
  margin-top: 64px; display: grid;
  grid-template-columns: repeat(3, 1fr);
  border: 1px solid var(--line); border-radius: 14px;
  overflow: hidden; background: var(--bg-card);
}
.pylon-cloud .tile {
  padding: 32px 28px; border-right: 1px solid var(--line);
  display: flex; flex-direction: column; gap: 10px;
}
.pylon-cloud .tile:last-child { border-right: none; }
.pylon-cloud .tile .ic {
  width: 36px; height: 36px; border-radius: 8px;
  background: var(--accent-soft); color: var(--accent-deep);
  display: inline-flex; align-items: center; justify-content: center;
  font-family: "Geist Mono", monospace; font-size: 14px; font-weight: 600;
}
.pylon-cloud .tile h3 { font-size: 19px; font-weight: 600; margin: 0; letter-spacing: -.015em; color: var(--ink); }
.pylon-cloud .tile p { font-size: 14px; line-height: 1.55; color: var(--text-2); margin: 0; }
.pylon-cloud .tile code { font-family: "Geist Mono", monospace; font-size: 12.5px; background: var(--bg-alt); padding: 1px 5px; border-radius: 3px; color: var(--text); border: 1px solid var(--line); }

/* === PRICING === */
.pylon-cloud .pricing {
  margin-top: 64px; display: grid;
  grid-template-columns: repeat(3, 1fr); gap: 20px;
}
.pylon-cloud .plan {
  border: 1px solid var(--line-2); border-radius: 14px;
  background: var(--bg-card);
  padding: 32px 28px 28px;
  display: flex; flex-direction: column;
  position: relative;
}
.pylon-cloud .plan.featured {
  border-color: var(--accent);
  box-shadow: 0 0 0 1px var(--accent);
}
.pylon-cloud .plan .ribbon {
  position: absolute; top: -12px; left: 24px;
  background: var(--accent); color: #fff;
  font-family: "Geist Mono", monospace; font-size: 11px;
  letter-spacing: .08em; text-transform: uppercase; font-weight: 500;
  padding: 4px 10px; border-radius: 999px;
}
.pylon-cloud .plan .name { font-family: "Geist Mono", monospace; font-size: 12px; color: var(--text-3); text-transform: uppercase; letter-spacing: .12em; font-weight: 500; }
.pylon-cloud .plan .price { font-size: 44px; font-weight: 600; letter-spacing: -.03em; line-height: 1; margin: 14px 0 4px; color: var(--ink); }
.pylon-cloud .plan .price .per { font-size: 14px; color: var(--text-3); font-weight: 400; letter-spacing: 0; }
.pylon-cloud .plan .blurb { font-size: 14px; color: var(--text-2); line-height: 1.55; margin: 0 0 22px; min-height: 44px; }
.pylon-cloud .plan ul {
  list-style: none; padding: 0; margin: 0 0 26px;
  display: flex; flex-direction: column; gap: 10px;
  font-size: 14px; color: var(--text-2);
}
.pylon-cloud .plan ul li { display: grid; grid-template-columns: 16px 1fr; gap: 10px; align-items: baseline; line-height: 1.5; }
.pylon-cloud .plan ul li::before {
  content: ""; width: 14px; height: 14px; border-radius: 50%;
  background: var(--accent-soft); border: 1px solid var(--accent);
  position: relative; top: 1px;
}
.pylon-cloud .plan ul li b { color: var(--ink); font-weight: 500; }
.pylon-cloud .plan .cta { margin-top: auto; }
.pylon-cloud .plan .cta .btn { width: 100%; justify-content: center; padding: 11px 14px; }

/* === COMPARE STRIP === */
.pylon-cloud .lanes {
  margin-top: 56px; display: grid;
  grid-template-columns: repeat(3, 1fr);
  border: 1px solid var(--line); border-radius: 14px;
  overflow: hidden; background: var(--bg-card);
}
.pylon-cloud .lane {
  padding: 28px 26px; border-right: 1px solid var(--line);
  display: flex; flex-direction: column; gap: 10px;
}
.pylon-cloud .lane:last-child { border-right: none; }
.pylon-cloud .lane .top { display: flex; justify-content: space-between; font-family: "Geist Mono", monospace; font-size: 11px; color: var(--text-3); letter-spacing: .08em; text-transform: uppercase; }
.pylon-cloud .lane .top b { color: var(--accent); font-weight: 500; }
.pylon-cloud .lane h4 { font-size: 21px; font-weight: 600; letter-spacing: -.02em; margin: 4px 0 4px; color: var(--ink); }
.pylon-cloud .lane .cmd { font-family: "Geist Mono", monospace; font-size: 12.5px; background: var(--code-bg); color: var(--code-green); padding: 7px 11px; border-radius: 6px; align-self: flex-start; margin: 4px 0; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pylon-cloud .lane p { font-size: 13.5px; color: var(--text-2); line-height: 1.5; margin: 0; }

/* === FAQ === */
.pylon-cloud .faq { margin-top: 56px; display: flex; flex-direction: column; gap: 0; border-top: 1px solid var(--line); }
.pylon-cloud .faq details { border-bottom: 1px solid var(--line); padding: 22px 0; }
.pylon-cloud .faq summary {
  list-style: none; cursor: pointer;
  display: flex; justify-content: space-between; align-items: center; gap: 24px;
  font-size: 17px; font-weight: 500; letter-spacing: -.01em; color: var(--ink);
}
.pylon-cloud .faq summary::-webkit-details-marker { display: none; }
.pylon-cloud .faq summary::after {
  content: "+"; font-family: "Geist Mono", monospace; font-size: 20px;
  color: var(--text-3); transition: transform .2s ease;
}
.pylon-cloud .faq details[open] summary::after { content: "−"; }
.pylon-cloud .faq p { font-size: 14.5px; color: var(--text-2); line-height: 1.6; margin: 14px 0 0; max-width: 680px; }
.pylon-cloud .faq p code { font-family: "Geist Mono", monospace; font-size: 12.5px; background: var(--bg-alt); padding: 1px 5px; border-radius: 3px; color: var(--text); border: 1px solid var(--line); }

/* === CTA === */
.pylon-cloud .cta-block { position: relative; padding: 96px 0 112px; text-align: left; overflow: hidden; background: var(--ink); color: #f3f3f4; }
.pylon-cloud .cta-block .bg-grid {
  position: absolute; inset: 0;
  background-image:
    linear-gradient(rgba(255,255,255,.03) 1px, transparent 1px),
    linear-gradient(90deg, rgba(255,255,255,.03) 1px, transparent 1px);
  background-size: 56px 56px;
  mask-image: radial-gradient(900px 500px at 80% 50%, #000 30%, transparent 75%);
  -webkit-mask-image: radial-gradient(900px 500px at 80% 50%, #000 30%, transparent 75%);
}
.pylon-cloud .cta-block .glow { position: absolute; right: -200px; top: -100px; width: 600px; height: 600px; background: radial-gradient(circle, rgba(255,91,31,.18), transparent 60%); pointer-events: none; }
.pylon-cloud .cta-block .inner { position: relative; max-width: 880px; }
.pylon-cloud .cta-block .eyebrow { color: var(--accent); }
.pylon-cloud .cta-block .eyebrow::before { background: var(--accent); }
.pylon-cloud .cta-block h2 { font-size: clamp(40px, 5vw, 68px); letter-spacing: -.04em; line-height: 1.02; font-weight: 600; margin: 18px 0; color: #fff; }
.pylon-cloud .cta-block h2 .serif { color: rgba(255,255,255,.55); }
.pylon-cloud .cta-block p { font-size: 18px; color: rgba(255,255,255,.65); max-width: 560px; line-height: 1.5; margin: 0 0 32px; }
.pylon-cloud .cta-block .ctas { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
.pylon-cloud .cta-block .ctas .term-pill { background: rgba(255,255,255,.04); border-color: rgba(255,255,255,.1); color: #f3f3f4; }
.pylon-cloud .cta-block .ctas .term-pill .prompt { color: rgba(255,255,255,.5); }
.pylon-cloud .cta-block .ctas .term-pill .copy { background: rgba(255,255,255,.08); color: rgba(255,255,255,.7); }

/* === FOOTER === */
.pylon-cloud footer { padding: 48px 0 32px; background: var(--bg); }
.pylon-cloud .foot {
  display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap;
  gap: 16px;
  font-family: "Geist Mono", monospace; font-size: 11.5px; color: var(--text-3);
}
.pylon-cloud .foot .status { display: inline-flex; align-items: center; gap: 8px; }
.pylon-cloud .foot .status::before { content: ""; width: 7px; height: 7px; border-radius: 50%; background: var(--pos); box-shadow: 0 0 0 3px var(--pos-soft); }
.pylon-cloud .foot a { color: var(--text-3); }
.pylon-cloud .foot a:hover { color: var(--text); }
.pylon-cloud .foot .dot { color: var(--line-2); margin: 0 4px; }

/* === RESPONSIVE === */
@media (max-width: 1100px) {
  .pylon-cloud .nav-links { display: none; }
  .pylon-cloud .tiles, .pylon-cloud .pricing, .pylon-cloud .lanes { grid-template-columns: 1fr 1fr; }
  .pylon-cloud .lane:nth-child(2) { border-right: none; }
  .pylon-cloud .lane:nth-child(3) { border-top: 1px solid var(--line); grid-column: 1 / -1; }
  .pylon-cloud .tile:nth-child(2) { border-right: none; }
  .pylon-cloud .tile:nth-child(3) { border-top: 1px solid var(--line); grid-column: 1 / -1; }
  .pylon-cloud .hero-meta { grid-template-columns: 1fr 1fr; gap: 18px 24px; }
}

@media (max-width: 720px) {
  .pylon-cloud .shell { padding: 0 20px; }

  .pylon-cloud .nav-cta .btn.ghost { display: none; }
  .pylon-cloud .nav-cta .btn { padding: 8px 12px; font-size: 13px; }
  .pylon-cloud .brand .tag { display: none; }

  .pylon-cloud .hero { padding-top: 56px; }
  .pylon-cloud h1.h1 { font-size: 42px; line-height: 1.02; max-width: 12ch; margin: 22px 0 18px; }
  .pylon-cloud .hero p.lede { font-size: 17px; line-height: 1.52; margin-bottom: 26px; }

  .pylon-cloud .hero-ctas { flex-direction: column; align-items: stretch; }
  .pylon-cloud .hero-ctas .btn,
  .pylon-cloud .hero-ctas .term-pill,
  .pylon-cloud .cta-block .ctas .btn,
  .pylon-cloud .cta-block .ctas .term-pill {
    width: 100%; justify-content: center; min-height: 42px;
  }

  .pylon-cloud .hero-meta { grid-template-columns: 1fr 1fr; margin-top: 40px; gap: 16px 12px; padding: 16px 0; }
  .pylon-cloud .hero-meta .item .v { font-size: 15px; }

  .pylon-cloud .deploy-frame { margin-top: 40px; border-radius: 12px; }
  .pylon-cloud .deploy-body { padding: 16px; font-size: 12px; line-height: 1.75; overflow-x: auto; }
  .pylon-cloud .deploy-body > div { white-space: nowrap; }

  .pylon-cloud section.block { padding: 72px 0; }
  .pylon-cloud h2.h2 { font-size: 34px; line-height: 1.08; }
  .pylon-cloud .lede { font-size: 16px; }

  .pylon-cloud .tiles, .pylon-cloud .pricing, .pylon-cloud .lanes { grid-template-columns: 1fr; margin-top: 44px; }
  .pylon-cloud .tile, .pylon-cloud .lane { border-right: none !important; border-bottom: 1px solid var(--line); padding: 24px 22px; }
  .pylon-cloud .tile:last-child, .pylon-cloud .lane:last-child { border-bottom: none; }
  .pylon-cloud .lane:nth-child(3) { border-top: none; grid-column: auto; }

  .pylon-cloud .pricing { gap: 16px; }
  .pylon-cloud .plan { padding: 26px 22px 22px; }
  .pylon-cloud .plan .price { font-size: 38px; }
  .pylon-cloud .plan .blurb { min-height: 0; }

  .pylon-cloud .faq summary { font-size: 15.5px; gap: 16px; }
  .pylon-cloud .faq p { font-size: 14px; }

  .pylon-cloud .cta-block { padding: 72px 0 80px; }
  .pylon-cloud .cta-block .ctas { flex-direction: column; align-items: stretch; }

  .pylon-cloud .foot { flex-direction: column; align-items: flex-start; gap: 8px; }
  .pylon-cloud .foot .dot { display: none; }
}

@media (max-width: 420px) {
  .pylon-cloud .shell { padding: 0 16px; }
  .pylon-cloud h1.h1 { font-size: 38px; }
  .pylon-cloud .hero-meta { grid-template-columns: 1fr; }
  .pylon-cloud h2.h2 { font-size: 30px; }
}

@media (prefers-reduced-motion: reduce) {
  .pylon-cloud *,
  .pylon-cloud *::before,
  .pylon-cloud *::after {
    animation-duration: .01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: .01ms !important;
  }
}
`;

function CopyCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className="term-pill mono"
      onClick={() => {
        void navigator.clipboard.writeText(command).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        });
      }}
      aria-label={`Copy command: ${command}`}
    >
      <span className="prompt">$</span> <span className="cmd-text">{command}</span>{" "}
      <span className="copy">{copied ? "✓" : "⧉"}</span>
    </button>
  );
}

export function CloudPage() {
  return (
    <>
      <link rel="preconnect" href="https://fonts.googleapis.com" />
      <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="" />
      <link
        href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600;700&family=Geist+Mono:wght@400;500&family=Instrument+Serif:ital@0;1&display=swap"
        rel="stylesheet"
      />
      <style dangerouslySetInnerHTML={{ __html: CLOUD_CSS }} />
      <div className="pylon-cloud">
        {/* NAV */}
        <nav className="nav">
          <div className="shell nav-inner">
            <Link className="brand" href="/">
              <PylonMark size={20} style={{ color: "var(--ink)" }} />
              Pylon<span className="tag">CLOUD</span>
            </Link>
            <ul className="nav-links">
              <li><a href="#why">Why managed</a></li>
              <li><a href="#pricing">Pricing</a></li>
              <li><a href="#faq">FAQ</a></li>
              <li><a href="https://docs.pylonsync.com">Docs</a></li>
              <li><Link href="/">← Pylon</Link></li>
            </ul>
            <div className="nav-cta">
              <Link className="btn ghost" href="https://cloud.pylonsync.com/login">Sign in</Link>
              <Link className="btn dark" href="https://cloud.pylonsync.com/signup">Start free →</Link>
            </div>
          </div>
        </nav>

        {/* HERO */}
        <header className="hero">
          <div className="hero-grid" />
          <div className="shell">
            <div className="hero-text">
              <span className="hero-tag">
                <span className="pill">FREE TIER</span>
                One project, no card, no time limit
              </span>
              <h1 className="h1">
                Managed Pylon, <span className="serif">escape hatch included.</span>
              </h1>
              <p className="lede">
                One command deploys your local Pylon backend to a global edge — <b>same binary, same APIs.</b>{" "}
                Free until you outgrow it. Move to your own AWS account whenever you want.
              </p>
              <div className="hero-ctas">
                <Link className="btn accent" href="https://cloud.pylonsync.com/signup">Start free →</Link>
                <CopyCommand command="pylon deploy" />
                <a className="btn ghost" href="https://docs.pylonsync.com/cloud">Read the docs →</a>
              </div>

              <div className="hero-meta">
                <div className="item"><div className="k">Cold start</div><div className="v">~80<span className="serif">ms</span></div></div>
                <div className="item"><div className="k">Regions</div><div className="v">12<span className="serif">+</span> · global</div></div>
                <div className="item"><div className="k">Free tier</div><div className="v">1 project · 1<span className="serif">GB</span></div></div>
                <div className="item"><div className="k">Compliance</div><div className="v">SOC&nbsp;2 <span className="serif">in flight</span></div></div>
              </div>

              <div className="deploy-frame">
                <div className="deploy-head">
                  <div className="dots"><i /><i /><i /></div>
                  <span>my-app — pylon deploy</span>
                </div>
                <div className="deploy-body">
                  <div><span className="pr">❯</span> pylon deploy</div>
                  <div><span className="dim">  Building binary for linux/arm64...</span></div>
                  <div><span className="ok">  ✓ Built · 28.4 MB · 11.2s</span></div>
                  <div><span className="ok">  ✓ Schema synced · 0 conflicts</span></div>
                  <div><span className="ok">  ✓ Deployed to iad, lhr, fra, syd</span></div>
                  <div><span className="dim">  → live at </span><span className="url">https://acme.pylonsync.app</span></div>
                  <div>&nbsp;</div>
                  <div><span className="dim">  # Promote to your own domain when ready:</span></div>
                  <div><span className="dim">  # </span>pylon domains add acme.com</div>
                  <div><span className="pr">❯</span><span className="blink" /></div>
                </div>
              </div>
            </div>
          </div>
        </header>

        {/* WHY MANAGED */}
        <section className="block" id="why">
          <div className="shell">
            <div className="eyebrow">Why managed</div>
            <h2 className="h2">All the framework. <span className="serif">None of the ops.</span></h2>
            <p className="lede">
              You wanted to ship a product, not run Postgres. Pylon Cloud takes the binary you built locally and runs it — with hot deploys, persistent storage, and the same dashboard you saw in <code style={{ fontFamily: "'Geist Mono',monospace", fontSize: 13, background: "var(--bg-alt)", padding: "1px 5px", borderRadius: 3, border: "1px solid var(--line)" }}>pylon dev</code>.
            </p>

            <div className="tiles">
              <div className="tile">
                <div className="ic">↑</div>
                <h3>One command, one URL.</h3>
                <p><code>pylon deploy</code> ships the same binary you ran locally. Live HTTPS endpoint in under 30 seconds. Atomic rollouts, instant rollback.</p>
              </div>
              <div className="tile">
                <div className="ic">⌖</div>
                <h3>Global edge, durable storage.</h3>
                <p>WebSockets terminate at the nearest region. Reads served from a local mirror. Postgres-backed primary with point-in-time recovery on every plan.</p>
              </div>
              <div className="tile">
                <div className="ic">⇄</div>
                <h3>Walk away whenever.</h3>
                <p>Same binary runs on a VPS, in your AWS account, or anywhere Docker runs. Export your data with <code>pylon export</code>. No proprietary runtime.</p>
              </div>
            </div>
          </div>
        </section>

        {/* PRICING */}
        <section className="block" id="pricing" style={{ background: "var(--bg-alt)" }}>
          <div className="shell">
            <div className="eyebrow">Pricing</div>
            <h2 className="h2">Free until <span className="serif">you outgrow it.</span></h2>
            <p className="lede">No seat tax. No per-WebSocket fees. Pricing scales with the storage and compute you actually use.</p>

            <div className="pricing">
              <div className="plan">
                <div className="name">Free</div>
                <div className="price">$0<span className="per"> / forever</span></div>
                <p className="blurb">For weekend projects, internal tools, and your first user.</p>
                <ul>
                  <li><span><b>1 project</b> · 1 GB storage</span></li>
                  <li><span><b>10k</b> monthly active users</span></li>
                  <li><span>Community support</span></li>
                  <li><span>Sleeps after 30 days idle</span></li>
                </ul>
                <div className="cta">
                  <Link className="btn line" href="https://cloud.pylonsync.com/signup">Start free</Link>
                </div>
              </div>

              <div className="plan featured">
                <span className="ribbon">Most teams</span>
                <div className="name">Pro</div>
                <div className="price">$25<span className="per"> / project / mo</span></div>
                <p className="blurb">Production apps with predictable cost.</p>
                <ul>
                  <li><span><b>Unlimited</b> projects</span></li>
                  <li><span><b>50 GB</b> storage included, $0.20/GB after</span></li>
                  <li><span><b>500k</b> MAU included</span></li>
                  <li><span>Custom domains, daily backups</span></li>
                  <li><span>Email support, 24h response</span></li>
                </ul>
                <div className="cta">
                  <Link className="btn accent" href="https://cloud.pylonsync.com/signup?plan=pro">Start Pro trial</Link>
                </div>
              </div>

              <div className="plan">
                <div className="name">Scale</div>
                <div className="price">Custom</div>
                <p className="blurb">For teams running Pylon as critical infrastructure.</p>
                <ul>
                  <li><span><b>SOC 2</b>, BAA, dedicated VPC</span></li>
                  <li><span>Bring-your-own-cloud (AWS, GCP)</span></li>
                  <li><span>SSO, audit log export</span></li>
                  <li><span>SLA, on-call escalation</span></li>
                  <li><span>Migration support from Convex / Supabase</span></li>
                </ul>
                <div className="cta">
                  <Link className="btn line" href="mailto:cloud@pylonsync.com?subject=Pylon%20Cloud%20Scale">Talk to us</Link>
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* DEPLOY LANES */}
        <section className="block">
          <div className="shell">
            <div className="eyebrow">Same binary</div>
            <h2 className="h2">Start on Cloud. <span className="serif">Or don&apos;t.</span></h2>
            <p className="lede">Pylon Cloud is one of four deploy targets. Pick the one that fits today — switch later without rewriting handlers.</p>

            <div className="lanes">
              <div className="lane">
                <div className="top"><span><b>02</b> · CLOUD</span><span>managed</span></div>
                <h4>Pylon Cloud</h4>
                <div className="cmd">$ pylon deploy</div>
                <p>This page. Free tier, global edge, daily backups. The fastest way from <code>pylon dev</code> to a public URL.</p>
              </div>
              <div className="lane">
                <div className="top"><span><b>03</b> · SELF-HOST</span><span>portable</span></div>
                <h4>Your VPS</h4>
                <div className="cmd">$ docker run pylon</div>
                <p>Run the same binary on Hetzner, Fly, Railway, or a Raspberry Pi in your closet. Systemd unit included.</p>
              </div>
              <div className="lane">
                <div className="top"><span><b>04</b> · AWS / GCP</span><span>terraform</span></div>
                <h4>Your cloud</h4>
                <div className="cmd">$ terraform apply</div>
                <p>Move into your own AWS account with Aurora, ALB, and your VPC. Reference Terraform module ships with the CLI.</p>
              </div>
            </div>
          </div>
        </section>

        {/* FAQ */}
        <section className="block" id="faq" style={{ background: "var(--bg-alt)" }}>
          <div className="shell">
            <div className="eyebrow">FAQ</div>
            <h2 className="h2">Things <span className="serif">people ask.</span></h2>

            <div className="faq">
              <details>
                <summary>What happens when I outgrow the free tier?</summary>
                <p>Your project keeps running. We email you a week before any limit, and you can upgrade in the dashboard or migrate to self-host with <code>pylon export</code> + <code>docker run</code>. Nothing gets deleted, nothing gets locked behind an upgrade gate.</p>
              </details>
              <details>
                <summary>Is the binary on Cloud the same as the one I run locally?</summary>
                <p>Yes — bit-for-bit. <code>pylon deploy</code> compiles your project into the same binary that ships on GitHub releases and runs against <code>pylon dev</code>. We don&apos;t inject runtime hooks, vendor proxies, or hosted-only APIs.</p>
              </details>
              <details>
                <summary>Can I bring my own database?</summary>
                <p>On Pro and Scale, yes — point <code>DATABASE_URL</code> at any Postgres (RDS, Neon, Supabase, your own VPS) and Pylon will use it as the primary. We still manage the runtime, you keep ownership of the data.</p>
              </details>
              <details>
                <summary>What does &quot;sleeps after 30 days idle&quot; mean?</summary>
                <p>Free-tier projects with no traffic for 30 consecutive days are paused. They wake up on the next request (~3s cold start). Pro and Scale projects never sleep.</p>
              </details>
              <details>
                <summary>How do migrations and backups work?</summary>
                <p>Schema migrations are applied atomically on deploy — same as <code>pylon dev</code>. Pro plans get daily snapshots with 30-day retention; Scale gets point-in-time recovery to any second in the last 14 days.</p>
              </details>
              <details>
                <summary>SOC 2, GDPR, HIPAA?</summary>
                <p>SOC 2 Type 1 is in audit, with Type 2 expected next quarter. GDPR data residency available on Scale (EU and US regions). HIPAA BAA available on Scale; reach out before deploying PHI.</p>
              </details>
            </div>
          </div>
        </section>

        {/* CTA */}
        <section className="cta-block">
          <div className="bg-grid" />
          <div className="glow" />
          <div className="shell">
            <div className="inner">
              <div className="eyebrow">Ship it</div>
              <h2>Deploy in the time <span className="serif">it takes to read this.</span></h2>
              <p>Sign in with GitHub. Create a project. Run <code style={{ fontFamily: "'Geist Mono',monospace", color: "#fff", background: "rgba(255,255,255,.08)", padding: "1px 8px", borderRadius: 4, border: "1px solid rgba(255,255,255,.12)" }}>pylon deploy</code>. That&apos;s the whole onboarding.</p>
              <div className="ctas">
                <Link className="btn accent" href="https://cloud.pylonsync.com/signup">Start free →</Link>
                <CopyCommand command="pylon deploy" />
                <a className="btn ghost" style={{ color: "rgba(255,255,255,.7)" }} href="https://docs.pylonsync.com/cloud">Read the docs</a>
              </div>
            </div>
          </div>
        </section>

        {/* FOOTER */}
        <footer>
          <div className="shell foot">
            <span>© 2026 Pylon Labs, Inc <span className="dot">·</span> MIT / Apache-2.0</span>
            <span><Link href="/">← Pylon framework</Link> <span className="dot">·</span> <a href="https://docs.pylonsync.com">Docs</a> <span className="dot">·</span> <a href="https://status.pylonsync.com">Status</a></span>
            <span className="status">All systems operational</span>
          </div>
        </footer>
      </div>
    </>
  );
}
