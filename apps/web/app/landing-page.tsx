"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { PylonMark } from "@/components/pylon-logo";

// Sourced from a Claude Design handoff (Pylon.html). The CSS is kept
// verbatim from the design — only the `:root`, `html/body`, and `*`
// global selectors were scoped to `.pylon-landing` so it doesn't fight
// any other styles that load with this Next app.
//
// Fonts (Geist + Geist Mono) are loaded via next/font from
// `layout.tsx`, exposed as CSS variables `--font-geist-sans`
// and `--font-geist-mono`. Self-hosted WOFF2, no Google Fonts
// roundtrip. The page is uniform Geist — earlier iterations
// had an Instrument Serif italic accent on headings, dropped
// 2026-05-21 in favor of a single typeface across the whole
// surface.

const DESIGN_CSS = `
/* Dark-mode landing. Pin html + body to the page bg so the
   canvas doesn't flash white on initial paint. */
html, body { background: #0a0a0c; color: #ededee; }

.pylon-landing {
  /* Dark-mode token set. The previous light tokens are inverted
     point-for-point: --bg goes near-black, --ink (the strongest
     text color, originally near-black) goes near-white, etc.
     The accent (purple) shifts ~one step brighter so it reads
     with enough contrast on the dark canvas — same hue, more
     luminance. The brand stays recognizable. */
  --bg: #0a0a0c;
  --bg-alt: #131318;
  --bg-card: #16161c;
  --ink: #fafafa;
  --ink-2: #ededee;
  --text: #ededee;
  --text-2: #a1a1aa;
  --text-3: #71717a;
  --line: rgba(255,255,255,.08);
  --line-2: rgba(255,255,255,.14);
  --accent: #a78bfa;
  --accent-soft: rgba(167,139,250,.12);
  --accent-deep: #c4b5fd;
  --pos: #4ade80;
  --pos-soft: rgba(74,222,128,.14);
  --code-bg: #0c0c0f;
  --code-text: #ededee;
  --code-mute: #71717a;
  --code-blue: #79b8ff;
  --code-purple: #c8a2ff;
  --code-green: #98e08c;
  --code-orange: #ffb86b;
  --code-red: #ff7b8a;
  --code-yellow: #ffd76b;
  /* Shadows in dark mode are mostly subtle highlights — a dark
     drop shadow on a dark surface disappears, so we use a
     near-black with low alpha + an inner highlight via the
     box-shadow second value. */
  --shadow-sm: 0 1px 2px rgba(0,0,0,.40), 0 0 0 1px rgba(255,255,255,.02);
  --shadow-md: 0 8px 24px -8px rgba(0,0,0,.50), 0 2px 6px rgba(0,0,0,.30);
  --shadow-lg: 0 24px 48px -16px rgba(0,0,0,.60), 0 4px 12px rgba(0,0,0,.30);
  background: var(--bg);
  color: var(--text);
  font-family: var(--font-geist-sans), -apple-system, system-ui, sans-serif;
  font-feature-settings: "ss01","cv11";
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  min-height: 100vh;
  overflow-x: hidden;
  -webkit-tap-highlight-color: rgba(139, 92, 246, .18);
}
.pylon-landing * { box-sizing: border-box; }
.pylon-landing .mono { font-family: var(--font-geist-mono), ui-monospace, monospace; }
.pylon-landing a { color: inherit; text-decoration: none; }
.pylon-landing button { font-family: inherit; cursor: pointer; }
.pylon-landing a, .pylon-landing button { touch-action: manipulation; }
.pylon-landing a:focus-visible, .pylon-landing button:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 3px;
}
.pylon-landing img { max-width: 100%; display: block; }

.pylon-landing .shell { max-width: 1280px; margin: 0 auto; padding: 0 32px; }
.pylon-landing .shell-wide { max-width: 1440px; margin: 0 auto; padding: 0 32px; }

/* === NAV === */
.pylon-landing .nav {
  position: sticky; top: 0; z-index: 50;
  background: rgba(10, 10, 12, .72);
  backdrop-filter: blur(14px) saturate(160%);
  -webkit-backdrop-filter: blur(14px) saturate(160%);
  border-bottom: 1px solid var(--line);
}
.pylon-landing .nav-inner { display: flex; align-items: center; justify-content: space-between; padding: 14px 0; }
.pylon-landing .brand { display: flex; align-items: center; gap: 9px; font-weight: 600; letter-spacing: -.02em; font-size: 16px; }
.pylon-landing .brand .logomark { width: 22px; height: 22px; position: relative; }
.pylon-landing .brand .logomark svg { width: 100%; height: 100%; display: block; }
.pylon-landing .nav-links { display: flex; gap: 4px; list-style: none; padding: 0; margin: 0; font-size: 13.5px; }
.pylon-landing .nav-links li a { display: inline-block; padding: 6px 12px; border-radius: 8px; color: var(--text-2); transition: color .15s ease, background .15s ease; }
.pylon-landing .nav-links li a:hover { color: var(--text); background: var(--bg-alt); }
.pylon-landing .nav-cta { display: flex; gap: 8px; align-items: center; }
/* Build-time-cached star count from the GitHub repo. Renders as
   icon + star + count. Falls back to a plain "GitHub" label when the
   build-time fetch fails (rate-limited, network blip, etc.) — the
   link still works, the visitor just doesn't see the count. */
.pylon-landing .nav-github {
  display: inline-flex; align-items: center; gap: 7px;
  padding: 6px 11px;
  border: 1px solid var(--line-2);
  border-radius: 8px;
  font-size: 13px; color: var(--text-2);
  background: var(--bg-card);
  transition: border-color .15s ease, color .15s ease, background .15s ease;
}
.pylon-landing .nav-github:hover {
  border-color: var(--ink);
  color: var(--ink);
}
.pylon-landing .nav-github svg { display: block; }
.pylon-landing .nav-github .nav-github-stars {
  font-family: var(--font-geist-mono), ui-monospace, monospace;
  font-size: 12px;
  letter-spacing: -.01em;
}
.pylon-landing .btn {
  display: inline-flex; align-items: center; gap: 8px;
  padding: 8px 14px; border-radius: 8px;
  font-size: 13.5px; font-weight: 500;
  border: 1px solid transparent;
  transition: background .15s ease, border-color .15s ease, color .15s ease, box-shadow .15s ease;
  font-feature-settings: "ss01";
  text-decoration: none;
  background: transparent; color: inherit;
}
.pylon-landing .btn.ghost { color: var(--text-2); }
.pylon-landing .btn.ghost:hover { color: var(--text); background: var(--bg-alt); }
.pylon-landing .btn.line { border-color: var(--line-2); color: var(--text); background: var(--bg-card); }
.pylon-landing .btn.line:hover { border-color: var(--ink); box-shadow: var(--shadow-sm); }
/* "dark" used to mean "high-contrast button vs. the cream
   canvas." In dark-mode --ink is now near-white, so the bg is
   right but the text needs to flip dark for legibility. The
   hover state pumps the bg toward pure white + adds the
   accent's purple ambient glow rather than a black shadow
   (which is invisible on dark anyway). */
.pylon-landing .btn.dark { background: var(--ink); color: #0a0a0c; border-color: var(--ink); }
.pylon-landing .btn.dark:hover { background: #fff; box-shadow: 0 4px 14px rgba(167,139,250,.22); }
.pylon-landing .btn.accent { background: var(--accent); color: #fff; border-color: var(--accent); }
.pylon-landing .btn.accent:hover { background: var(--accent-deep); border-color: var(--accent-deep); box-shadow: 0 4px 14px rgba(139,92,246,.28); }
.pylon-landing .kbd {
  display: inline-flex; align-items: center; justify-content: center;
  min-width: 18px; height: 18px; padding: 0 5px; border-radius: 4px;
  background: var(--bg-alt); color: var(--text-3); font-size: 11px;
  font-family: var(--font-geist-mono), ui-monospace, monospace; border: 1px solid var(--line-2);
  box-shadow: 0 1px 0 var(--line-2);
}

.pylon-landing .nav-burger {
  display: none;
  width: 36px; height: 36px;
  border: 1px solid var(--line-2); border-radius: 8px;
  background: var(--bg-card);
  align-items: center; justify-content: center;
  flex-direction: column; gap: 4px;
  padding: 0;
  flex-shrink: 0;
  transition: background .15s ease, border-color .15s ease;
}
.pylon-landing .nav-burger:hover { background: var(--bg-alt); }
.pylon-landing .nav-burger span {
  display: block; width: 16px; height: 1.5px;
  background: var(--ink); border-radius: 2px;
  transition: transform .2s ease, opacity .15s ease;
}
.pylon-landing .nav.menu-open .nav-burger span:nth-child(1) { transform: translateY(5.5px) rotate(45deg); }
.pylon-landing .nav.menu-open .nav-burger span:nth-child(2) { opacity: 0; }
.pylon-landing .nav.menu-open .nav-burger span:nth-child(3) { transform: translateY(-5.5px) rotate(-45deg); }

.pylon-landing .nav-sheet {
  display: none;
  position: absolute; top: 100%; left: 0; right: 0;
  background: var(--bg-card);
  border-bottom: 1px solid var(--line);
  box-shadow: var(--shadow-md);
  flex-direction: column;
  padding: 8px 20px 14px;
  z-index: 49;
}
.pylon-landing .nav-sheet a {
  display: block; padding: 12px 4px;
  font-size: 15px; color: var(--text);
  border-bottom: 1px solid var(--line);
}
.pylon-landing .nav-sheet a:last-child { border-bottom: none; }
.pylon-landing .nav-sheet .sheet-signin { color: var(--accent); font-weight: 500; }
.pylon-landing .nav.menu-open .nav-sheet { display: flex; }

/* === HERO === */
.pylon-landing .hero { padding: 64px 0 0; position: relative; overflow: hidden; }
.pylon-landing .hero-grid-bg {
  position: absolute; inset: 0;
  background-image:
    linear-gradient(var(--line) 1px, transparent 1px),
    linear-gradient(90deg, var(--line) 1px, transparent 1px);
  background-size: 56px 56px;
  background-position: -1px -1px;
  mask-image: radial-gradient(900px 500px at 50% 30%, #000 30%, transparent 80%);
  -webkit-mask-image: radial-gradient(900px 500px at 50% 30%, #000 30%, transparent 80%);
  opacity: .55;
  pointer-events: none;
}
.pylon-landing .hero-tag {
  display: inline-flex; align-items: center; gap: 8px;
  padding: 5px 12px 5px 6px;
  background: var(--bg-card);
  border: 1px solid var(--line-2);
  border-radius: 999px;
  font-size: 12.5px; color: var(--text-2);
  box-shadow: var(--shadow-sm);
}
.pylon-landing .hero-tag .pill {
  background: var(--accent-soft); color: var(--accent-deep);
  padding: 2px 9px; border-radius: 999px; font-size: 11px; font-weight: 600;
  letter-spacing: .02em;
}
.pylon-landing .hero-tag .arrow { color: var(--text-3); margin-left: 4px; }
.pylon-landing h1.h1 {
  font-size: clamp(40px, 4.8vw, 64px);
  line-height: 1.04;
  letter-spacing: -.04em;
  font-weight: 600;
  margin: 0 0 18px;
  color: var(--ink);
  /* Relaxed from 12ch — the new H1 doesn't fit in 12ch and forced
     an extra wrap that gave the hero column three stacked text
     lines. Two-line wrap reads better next to the product mock. */
  max-width: 16ch;
}

/* Rotating language slot inside the H1, rendered as a per-language
   tinted pill. Two-layer markup:
     .lang-ghost — invisible, holds the longest entry's text so
                   the pill's width stays fixed.
     .lang-word  — the visible word, stacked over the ghost in
                   the same grid cell, animated on each tick.
   The bg + fg + border-color come from inline style (set per
   tick from the LANGS table) so the chrome itself cross-fades
   when the language changes. */
.pylon-landing .lang-rotator {
  display: inline-grid;
  /* Single named cell — both .lang-ghost and .lang-word claim
     grid-area: stack, so they overlap perfectly without
     position: absolute math. */
  grid-template-areas: "stack";
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;  /* color set inline per tick */
  border-radius: 999px;
  /* Compress the pill vertically vs. the surrounding text — em
     units track the heading's clamp() so the pill stays
     proportional from mobile to desktop. */
  padding: 0.08em 0.42em 0.12em;
  font-weight: 600;
  letter-spacing: -.02em;
  /* Smooth crossfade of the chrome (bg + text + border) when
     the language changes. The inner word fade-up is a separate
     keyframe so the two compositions don't fight. */
  transition: background-color .42s cubic-bezier(.32, .08, .24, 1),
              color .42s cubic-bezier(.32, .08, .24, 1),
              border-color .42s cubic-bezier(.32, .08, .24, 1);
}
.pylon-landing .lang-ghost {
  grid-area: stack;
  visibility: hidden;
  pointer-events: none;
  /* Whitespace shouldn't collapse to zero in the ghost — the
     pill needs its full intrinsic width even though the
     content is invisible. */
  white-space: nowrap;
}
.pylon-landing .lang-word {
  grid-area: stack;
  white-space: nowrap;
  animation: lang-rotate .42s cubic-bezier(.32, .08, .24, 1);
}
@keyframes lang-rotate {
  0%   { opacity: 0; transform: translateY(-6px); }
  100% { opacity: 1; transform: translateY(0); }
}
@media (prefers-reduced-motion: reduce) {
  /* Skip the animations but still swap the text — same value,
     no visual motion. Chrome transition also disabled. */
  .pylon-landing .lang-word { animation: none; }
  .pylon-landing .lang-rotator { transition: none; }
}
.pylon-landing .hero p.lede {
  font-size: 18px; line-height: 1.45;
  max-width: 560px; color: var(--text-2);
  margin: 0 0 28px;
  letter-spacing: -.005em;
}
.pylon-landing .hero p.lede b { color: var(--ink); font-weight: 500; }

.pylon-landing .hero-ctas { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
/* Secondary path: visually demoted "or run locally" line that lives
   beneath the primary CTAs. Keeps the npm-create command available
   for power users without competing with the Cloud signup for the
   first scan. */
.pylon-landing .hero-secondary {
  display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
  margin-top: 18px;
}
.pylon-landing .hero-secondary-lede {
  font-size: 13px; color: var(--text-3);
}
.pylon-landing .hero-secondary .term-pill {
  padding: 5px 10px 5px 12px; font-size: 12.5px;
}
.pylon-landing .hero-secondary .term-pill .copy {
  width: 20px; height: 20px; flex: 0 0 20px;
}
.pylon-landing .cta-secondary {
  display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
  margin-top: 16px;
  font-size: 13px; color: rgba(255,255,255,.55);
}
/* "Live-ship" social proof badge. Pulls visitor attention without
   stealing it from the CTA — single-line pill, accent-coloured pulse
   on the left, "v0.3.x · shipped today" on the right. The whole thing
   links to the GitHub Releases page so a skeptic can verify. */
.pylon-landing .hero-ship-badge {
  display: inline-flex; align-items: center; gap: 8px;
  margin-top: 18px;
  padding: 6px 10px 6px 8px;
  border: 1px solid var(--line-2);
  border-radius: 999px;
  background: var(--bg-card);
  font-size: 12.5px; color: var(--text-2);
  text-decoration: none;
  box-shadow: var(--shadow-sm);
  transition: border-color .15s ease, color .15s ease, box-shadow .15s ease;
}
/* Eyebrow variant — sits above the H1 instead of below the CTAs.
   Tighter padding, smaller dot, no top margin so it composes as a
   true header eyebrow rather than a CTA-row sibling. */
.pylon-landing .hero-ship-badge--eyebrow {
  margin-top: 0;
  margin-bottom: 20px;
  padding: 4px 10px 4px 7px;
  font-size: 12px;
}
.pylon-landing .hero-ship-badge--eyebrow .dot {
  width: 5px; height: 5px;
  box-shadow: 0 0 0 2.5px rgba(22,163,74,.18);
}
.pylon-landing .hero-ship-badge:hover {
  border-color: var(--ink);
  color: var(--ink);
  box-shadow: var(--shadow-md);
}
.pylon-landing .hero-ship-badge .dot {
  width: 6px; height: 6px; border-radius: 999px;
  background: var(--pos);
  box-shadow: 0 0 0 3px rgba(22,163,74,.18);
}
.pylon-landing .hero-ship-badge .ver {
  font-family: var(--font-geist-mono), ui-monospace, monospace;
  font-size: 12px; color: var(--ink); font-weight: 500;
}
.pylon-landing .hero-ship-badge .sep { color: var(--text-3); }
.pylon-landing .hero-ship-badge .when { color: var(--text-2); }
.pylon-landing .hero-ship-badge .arrow { color: var(--text-3); margin-left: 2px; }
.pylon-landing .term-pill {
  display: inline-flex; align-items: center; gap: 10px;
  background: var(--bg-card); border: 1px solid var(--line-2);
  padding: 7px 12px 7px 14px; border-radius: 8px;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 13px;
  color: var(--text); box-shadow: var(--shadow-sm);
  max-width: 100%;
  min-width: 0;
}
.pylon-landing .term-pill .prompt { color: var(--text-3); flex-shrink: 0; }
.pylon-landing .term-pill .cmd-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pylon-landing .term-pill .copy {
  display: inline-flex; align-items: center; justify-content: center;
  width: 24px; height: 24px; border-radius: 5px; color: var(--text-3);
  background: var(--bg-alt);
  flex: 0 0 24px;
}

.pylon-landing .hero-text { text-align: left; position: relative; }

/* Hero layout — text column on the left, product mock on the right at
   desktop widths. The mock extends past the right edge of the standard
   shell so the dashboard's visible weight matches the text column's
   typographic weight. Visitors see the artifact above the fold instead
   of scrolling past empty whitespace to find it. Tightens the original
   hero from "text → 64px gap → big stat row → 72px gap → mock" down to
   "text and mock side by side, period." */
.pylon-landing .hero-layout {
  display: grid;
  grid-template-columns: minmax(420px, 1fr) minmax(540px, 1.15fr);
  gap: 56px;
  align-items: center;
  padding-bottom: 96px;
}
.pylon-landing .hero-layout .product-frame {
  margin-top: 0;
  max-width: none;
  /* Let the mock bleed past the right edge of the shell so it anchors
     to the viewport edge instead of sitting in a centered card. At a
     1920px viewport this widens the mock by ~352px (the gap from the
     shell's right edge to the viewport right edge, plus the shell's
     own 32px padding). On narrower viewports the calc clamps at 0
     and the mock stays inside the column.

     The mock keeps its rounded-corner pill shape — overflow is hidden
     and the right edge runs off the screen, which reads as "the
     surface continues past the page" — same pattern Stripe, Linear,
     Vercel use on their landing heroes. */
  width: calc(100% + max(0px, (100vw - 1280px) / 2 + 32px));
}
/* Inside the hero column the mock has roughly half the viewport to
   work with — the default 220px sidebar + 1fr main + 360px aside grid
   squashes the main column to a useless ~30px sliver. Drop the left
   workspace nav (generic dashboard chrome) and keep main + aside so
   the dashboard view AND the live-query code snippet — the actual
   proof of the H1's claim — both stay legible. The sidebar reappears
   when the layout collapses to single-column at <1100px and the mock
   has the full viewport again. */
.pylon-landing .hero-layout .product-body {
  grid-template-columns: 1fr 320px;
}
.pylon-landing .hero-layout .app-side { display: none; }
/* The inner table + metric typography is tuned for a full-width mock.
   At hero width the metric numbers clip ($49,288 → "$49,2") and the
   table tries to render five columns in ~360px of horizontal space —
   email crashes into total, status pill spills into the timestamp.
   Trim both: drop email + created from the table (keep Customer / Total
   / Status — the three columns that read at a glance), shrink the
   metric numerals + padding so the headline value fits. The full
   five-column table comes back in the single-column layout below
   1100px. */
.pylon-landing .hero-layout .app-main { padding: 18px 18px 20px; }
.pylon-landing .hero-layout .app-main h2.app-title {
  font-size: 17px;
  white-space: nowrap;
}
.pylon-landing .hero-layout .metric { padding: 10px 10px 8px; }
.pylon-landing .hero-layout .metric .num { font-size: 20px; }
.pylon-landing .hero-layout .metric .spark { width: 44px; height: 18px; }
.pylon-landing .hero-layout .tbl-head,
.pylon-landing .hero-layout .tbl-row {
  grid-template-columns: 1fr auto auto;
  gap: 10px;
}
.pylon-landing .hero-layout .tbl-head > *:nth-child(2),
.pylon-landing .hero-layout .tbl-head > *:nth-child(5),
.pylon-landing .hero-layout .tbl-row > *:nth-child(2),
.pylon-landing .hero-layout .tbl-row > *:nth-child(5) {
  display: none;
}

/* === HERO PRODUCT MOCK === */
.pylon-landing .product-frame {
  /* The product mock stays light-mode regardless of the page's
     theme — it's a screenshot-style demo of Pylon Cloud, which
     itself ships light. Re-declaring the design tokens locally
     overrides the inherited dark ones for every descendant
     (CSS variable cascade), so .metric, .tbl-row, .app-main,
     etc. all read against the light palette without needing
     per-rule overrides. */
  --bg: #fafaf9;
  --bg-alt: #f4f3f0;
  --bg-card: #ffffff;
  --ink: #0a0a0b;
  --ink-2: #1a1a1d;
  --text: #18181b;
  --text-2: #52525b;
  --text-3: #a1a1aa;
  --line: #e7e5e2;
  --line-2: #d4d4d0;
  --pos: #16a34a;
  --pos-soft: #e7f6ec;
  --shadow-sm: 0 1px 2px rgba(15,15,20,.04), 0 1px 1px rgba(15,15,20,.02);
  --shadow-md: 0 8px 24px -8px rgba(15,15,20,.10), 0 2px 6px rgba(15,15,20,.04);
  /* The drop shadow that ties the mock to the page needs to be
     visible against the dark canvas — use a deeper black drop
     instead of the page-level light shadow tokens. */
  margin-top: 72px; position: relative;
  border-radius: 16px;
  background: var(--bg-card);
  border: 1px solid var(--line-2);
  color: var(--text);
  box-shadow: 0 24px 48px -16px rgba(0,0,0,.65), 0 8px 20px rgba(0,0,0,.45);
  overflow: hidden;
}
.pylon-landing .product-frame::before {
  content: ""; position: absolute; inset: 0;
  background: linear-gradient(180deg, rgba(139,92,246,.04), transparent 30%);
  pointer-events: none;
}
.pylon-landing .product-chrome {
  display: flex; align-items: center; gap: 14px;
  padding: 12px 18px; border-bottom: 1px solid var(--line);
  background: linear-gradient(180deg, var(--bg-card), #fbfaf8);
}
.pylon-landing .product-chrome .dots { display: flex; gap: 6px; }
.pylon-landing .product-chrome .dots i { width: 11px; height: 11px; border-radius: 50%; background: #e1ddd5; display: inline-block; }
.pylon-landing .product-chrome .url {
  flex: 1; max-width: 460px; height: 28px; border-radius: 6px;
  background: var(--bg-alt); border: 1px solid var(--line);
  display: flex; align-items: center; gap: 8px; padding: 0 12px;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; color: var(--text-2);
}
.pylon-landing .product-chrome .url::before {
  content: ""; width: 10px; height: 10px; border-radius: 50%; background: var(--pos); flex-shrink: 0; box-shadow: 0 0 0 3px var(--pos-soft);
}
.pylon-landing .product-chrome .right { display: flex; gap: 10px; color: var(--text-3); font-size: 12.5px; align-items: center; }
.pylon-landing .product-chrome .right .badge {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 3px 8px; border-radius: 999px; background: var(--pos-soft); color: var(--pos);
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; font-weight: 500;
}
.pylon-landing .product-chrome .right .badge::before {
  content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--pos); animation: pylon-pulse 1.4s ease-in-out infinite;
}
@keyframes pylon-pulse { 50% { opacity: .4 } }

.pylon-landing .product-body { display: grid; grid-template-columns: 220px 1fr 360px; min-height: 520px; }
.pylon-landing .app-side {
  border-right: 1px solid var(--line);
  padding: 20px 14px;
  background: linear-gradient(180deg, #fbfaf7, var(--bg-card));
}
.pylon-landing .app-side .group { margin-bottom: 18px; }
.pylon-landing .app-side h6 {
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 10.5px;
  text-transform: uppercase; letter-spacing: .12em;
  color: var(--text-3); margin: 0 6px 8px; font-weight: 500;
}
.pylon-landing .app-side ul { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 1px; }
.pylon-landing .app-side li {
  display: flex; align-items: center; gap: 10px;
  padding: 6px 10px; border-radius: 6px;
  font-size: 13px; color: var(--text-2);
  cursor: default;
}
.pylon-landing .app-side li:hover { background: var(--bg-alt); }
.pylon-landing .app-side li.active { background: var(--ink); color: #fff; }
.pylon-landing .app-side li.active .dot { background: var(--accent); }
.pylon-landing .app-side li .glyph { width: 14px; height: 14px; flex-shrink: 0; opacity: .6; background: currentColor; border-radius: 3px; }
.pylon-landing .app-side li.active .glyph { opacity: 1; }
.pylon-landing .app-side li .count { margin-left: auto; font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; color: var(--text-3); }
.pylon-landing .app-side li.active .count { color: rgba(255,255,255,.5); }

.pylon-landing .app-main { padding: 22px 26px; min-width: 0; }
.pylon-landing .app-main .crumb { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11.5px; color: var(--text-3); display: flex; align-items: center; gap: 6px; }
.pylon-landing .app-main .crumb b { color: var(--text); font-weight: 500; }
.pylon-landing .app-main h2.app-title {
  font-size: 22px; letter-spacing: -.02em; font-weight: 600;
  margin: 6px 0 18px; display: flex; align-items: center; gap: 10px;
}
.pylon-landing .app-main h2.app-title .live {
  display: inline-flex; align-items: center; gap: 6px;
  background: var(--pos-soft); color: var(--pos);
  padding: 3px 9px; border-radius: 999px;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; font-weight: 500;
  letter-spacing: .02em;
}
.pylon-landing .app-main h2.app-title .live::before {
  content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--pos); animation: pylon-pulse 1.4s ease-in-out infinite;
}

.pylon-landing .metrics { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin-bottom: 18px; }
.pylon-landing .metric {
  border: 1px solid var(--line); border-radius: 10px;
  padding: 14px 14px 12px; background: var(--bg-card);
  position: relative; overflow: hidden;
}
.pylon-landing .metric .label { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; color: var(--text-3); text-transform: uppercase; letter-spacing: .08em; }
.pylon-landing .metric .num { font-size: 26px; font-weight: 600; letter-spacing: -.02em; margin-top: 6px; }
.pylon-landing .metric .delta { font-size: 11.5px; color: var(--pos); margin-top: 2px; font-family: var(--font-geist-mono), ui-monospace, monospace; }
.pylon-landing .metric .spark { position: absolute; right: 10px; bottom: 10px; width: 64px; height: 22px; }

.pylon-landing .tbl { border: 1px solid var(--line); border-radius: 10px; overflow: hidden; background: var(--bg-card); }
.pylon-landing .tbl-head {
  display: grid; grid-template-columns: 1.4fr 1.2fr .8fr .8fr .8fr;
  padding: 10px 14px; background: var(--bg-alt);
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; color: var(--text-3);
  text-transform: uppercase; letter-spacing: .08em;
  border-bottom: 1px solid var(--line);
}
.pylon-landing .tbl-row {
  display: grid; grid-template-columns: 1.4fr 1.2fr .8fr .8fr .8fr;
  padding: 11px 14px; font-size: 13px; align-items: center;
  border-bottom: 1px solid var(--line);
  transition: background .15s ease;
}
.pylon-landing .tbl-row:last-child { border-bottom: none; }
.pylon-landing .tbl-row:hover { background: #fdfcfa; }
.pylon-landing .tbl-row.flash { animation: pylon-flashRow 1.2s ease-out; }
@keyframes pylon-flashRow {
  0% { background: var(--accent-soft); }
  100% { background: transparent; }
}
.pylon-landing .tbl-head > *, .pylon-landing .tbl-row > * { min-width: 0; }
.pylon-landing .tbl-row .id { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pylon-landing .tbl-row .name { display: flex; align-items: center; gap: 8px; min-width: 0; }
.pylon-landing .tbl-row .avatar { width: 22px; height: 22px; border-radius: 50%; background: var(--bg-alt); border: 1px solid var(--line); display: inline-flex; align-items: center; justify-content: center; font-size: 10px; color: var(--text-2); font-weight: 500; }
.pylon-landing .status-pill {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 2px 8px; border-radius: 999px;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; font-weight: 500;
}
.pylon-landing .status-pill.paid { background: var(--pos-soft); color: var(--pos); }
.pylon-landing .status-pill.pending { background: #fff7e6; color: #b45309; }
.pylon-landing .status-pill.failed { background: #fee4e2; color: #b42318; }

.pylon-landing .app-aside {
  border-left: 1px solid var(--line);
  padding: 18px 18px 22px;
  background: linear-gradient(180deg, var(--bg-card), #fbfaf7);
  display: flex; flex-direction: column; gap: 16px;
}
.pylon-landing .aside-title { display: flex; align-items: center; gap: 8px; font-size: 13px; font-weight: 500; }
.pylon-landing .aside-title .ws { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 10.5px; color: var(--text-3); margin-left: auto; background: var(--bg-alt); padding: 2px 7px; border-radius: 4px; }
.pylon-landing .code-mini {
  background: var(--code-bg); color: var(--code-text);
  border-radius: 10px; padding: 14px;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; line-height: 1.65;
  border: 1px solid #1c1c22;
  position: relative; overflow: hidden;
}
.pylon-landing .code-mini .file {
  display: flex; justify-content: space-between; padding-bottom: 10px;
  border-bottom: 1px solid #1c1c22;
  font-size: 11px; color: var(--code-mute); margin-bottom: 12px; letter-spacing: .04em;
}
.pylon-landing .code-mini .file b { color: #d4d4d8; font-weight: 500; }
.pylon-landing .code-mini pre { margin: 0; white-space: pre-wrap; }
.pylon-landing .k1 { color: var(--code-purple); }
.pylon-landing .k2 { color: var(--code-blue); }
.pylon-landing .s1 { color: var(--code-green); }
.pylon-landing .f1 { color: var(--code-orange); }
.pylon-landing .c1 { color: var(--code-mute); font-style: italic; }
.pylon-landing .n1 { color: var(--code-yellow); }
.pylon-landing .t1 { color: #ffd1b8; }

.pylon-landing .events-panel { border: 1px solid var(--line); border-radius: 10px; background: var(--bg-card); padding: 12px 14px; }
.pylon-landing .events-panel .head {
  display: flex; justify-content: space-between; align-items: center;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px;
  color: var(--text-3); text-transform: uppercase; letter-spacing: .08em;
  margin-bottom: 10px;
}
.pylon-landing .events-panel .head b { color: var(--text); font-weight: 500; }
.pylon-landing .event {
  display: grid; grid-template-columns: 56px 1fr auto;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11.5px;
  padding: 4px 0; gap: 10px; align-items: baseline;
  border-bottom: 1px dashed var(--line);
}
.pylon-landing .event:last-child { border-bottom: none; }
.pylon-landing .event .t { color: var(--text-3); }
.pylon-landing .event .k { color: var(--text-2); }
.pylon-landing .event .k em { color: var(--accent); font-style: normal; }
.pylon-landing .event .v { color: var(--text); font-weight: 500; }

.pylon-landing .logos { margin-top: 96px; padding: 36px 0; border-top: 1px solid var(--line); border-bottom: 1px solid var(--line); }
.pylon-landing .logos .label { text-align: center; font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; letter-spacing: .12em; color: var(--text-3); text-transform: uppercase; margin-bottom: 28px; }
.pylon-landing .logos .row { display: grid; grid-template-columns: repeat(6, 1fr); gap: 36px; align-items: center; }
.pylon-landing .logo-faux { text-align: center; font-weight: 600; font-size: 16px; color: var(--text-3); letter-spacing: -.02em; opacity: .85; display: flex; align-items: center; justify-content: center; gap: 8px; }
.pylon-landing .logo-faux .glyph { width: 18px; height: 18px; border-radius: 4px; background: currentColor; opacity: .35; }

/* === SECTIONS === */
.pylon-landing section.block { padding: 88px 0; position: relative; border-bottom: 1px solid var(--line); }
.pylon-landing .eyebrow {
  display: inline-flex; align-items: center; gap: 8px;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11.5px;
  color: var(--accent); text-transform: uppercase; letter-spacing: .14em; font-weight: 500;
}
.pylon-landing .eyebrow::before { content: ""; width: 14px; height: 1px; background: var(--accent); }
.pylon-landing h2.h2 {
  font-size: clamp(32px, 3.6vw, 48px);
  letter-spacing: -.03em; line-height: 1.04;
  font-weight: 600; color: var(--ink);
  margin: 12px 0 14px; max-width: 22ch;
}
.pylon-landing .section-lede { font-size: 17px; color: var(--text-2); line-height: 1.5; max-width: 620px; letter-spacing: -.005em; }

/* === FEATURES === */
.pylon-landing .feature-row { display: grid; grid-template-columns: 1fr 1.2fr; gap: 56px; align-items: center; margin-top: 56px; }
.pylon-landing .feature-row.flip { grid-template-columns: 1.2fr 1fr; }
.pylon-landing .feature-row.flip .feature-copy { order: 2; }
.pylon-landing .feature-row.flip .feature-art { order: 1; }
.pylon-landing .feature-copy h3 {
  font-size: 26px; letter-spacing: -.025em; font-weight: 600;
  margin: 10px 0 12px; color: var(--ink); line-height: 1.1;
}
.pylon-landing .feature-copy p { font-size: 16px; line-height: 1.55; color: var(--text-2); margin: 0 0 24px; }
.pylon-landing .feature-bullets { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 14px; }
.pylon-landing .feature-bullets li { display: grid; grid-template-columns: 22px 1fr; gap: 12px; font-size: 14.5px; color: var(--text-2); line-height: 1.5; }
.pylon-landing .feature-bullets li b { color: var(--ink); font-weight: 500; }
.pylon-landing .feature-bullets li::before {
  content: ""; width: 16px; height: 16px;
  border-radius: 50%;
  background: var(--accent-soft);
  border: 1px solid var(--accent);
  position: relative;
  margin-top: 2px;
}
.pylon-landing .tiny-tag { display: inline-flex; align-items: center; gap: 6px; font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; color: var(--accent); letter-spacing: .08em; text-transform: uppercase; }

.pylon-landing .art { border: 1px solid var(--line-2); border-radius: 14px; background: var(--bg-card); box-shadow: var(--shadow-md); overflow: hidden; position: relative; }

/* schema editor art */
.pylon-landing .schema-art { padding: 0; }
.pylon-landing .schema-tabs { display: flex; border-bottom: 1px solid var(--line); background: var(--bg-alt); padding: 0 12px; }
.pylon-landing .schema-tabs .tab { padding: 10px 14px; font-size: 12px; font-family: var(--font-geist-mono), ui-monospace, monospace; color: var(--text-3); border-bottom: 2px solid transparent; }
.pylon-landing .schema-tabs .tab.on { color: var(--text); border-color: var(--accent); background: var(--bg-card); position: relative; top: 1px; }
.pylon-landing .schema-body { display: grid; grid-template-columns: 36px 1fr; min-height: 360px; }
.pylon-landing .schema-gutter { background: var(--bg-alt); border-right: 1px solid var(--line); font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; color: var(--text-3); padding: 14px 8px; text-align: right; line-height: 1.65; }
.pylon-landing .schema-code { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12.5px; line-height: 1.65; padding: 14px 16px; color: var(--text); background: var(--bg-card); }
.pylon-landing .schema-code pre { overflow-x: auto; }
.pylon-landing .schema-code .k { color: #a155b9; }
.pylon-landing .schema-code .s { color: #2e7d32; }
.pylon-landing .schema-code .t { color: #7C3AED; }
.pylon-landing .schema-code .c { color: var(--text-3); font-style: italic; }
.pylon-landing .schema-code .b { color: var(--ink); font-weight: 500; }
.pylon-landing .schema-code .anno { display: inline-block; padding: 0 6px; border-radius: 4px; background: var(--accent-soft); color: var(--accent-deep); margin-left: 6px; font-size: 11px; transform: translateY(-1px); }

/* live query art */
.pylon-landing .live-art { padding: 22px 22px 24px; }
.pylon-landing .live-art h5 { margin: 0 0 12px; font-size: 12px; font-family: var(--font-geist-mono), ui-monospace, monospace; letter-spacing: .1em; text-transform: uppercase; color: var(--text-3); display: flex; align-items: center; gap: 8px; }
.pylon-landing .live-art h5 .pulse { width: 7px; height: 7px; border-radius: 50%; background: var(--pos); box-shadow: 0 0 0 3px var(--pos-soft); animation: pylon-pulse 1.4s infinite; }
.pylon-landing .live-art .chart {
  height: 160px; border-bottom: 1px solid var(--line);
  position: relative; padding-bottom: 4px;
  background: linear-gradient(transparent 49.5%, rgba(231,229,226,.6) 50%, transparent 50.5%) 0 0/100% 25%;
}
.pylon-landing .live-art .chart svg { width: 100%; height: 100%; }
.pylon-landing .live-art .legend { display: flex; justify-content: space-between; padding: 10px 0 0; font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; color: var(--text-3); }
.pylon-landing .live-art .row-set { margin-top: 14px; display: grid; gap: 6px; }
.pylon-landing .live-art .row-set .r { display: grid; grid-template-columns: 80px 1fr 70px; font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; padding: 6px 10px; border-radius: 6px; background: var(--bg-alt); align-items: center; }
.pylon-landing .live-art .row-set .r .id { color: var(--text-2); }
.pylon-landing .live-art .row-set .r .lbl { color: var(--text); }
.pylon-landing .live-art .row-set .r .v { color: var(--accent); text-align: right; font-weight: 500; }

/* policies art */
.pylon-landing .policy-art { padding: 22px; }
.pylon-landing .policy-art .pcard { border: 1px solid var(--line); border-radius: 8px; padding: 12px 14px; margin-bottom: 10px; display: grid; grid-template-columns: 22px 1fr auto; gap: 12px; align-items: center; background: var(--bg-card); }
.pylon-landing .policy-art .pcard .ic { width: 22px; height: 22px; border-radius: 6px; background: var(--accent-soft); color: var(--accent-deep); display: inline-flex; align-items: center; justify-content: center; font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; font-weight: 600; }
.pylon-landing .policy-art .pcard .name { font-weight: 500; font-size: 14px; }
.pylon-landing .policy-art .pcard .rule { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; color: var(--text-2); margin-top: 2px; }
.pylon-landing .policy-art .pcard .rule em { color: var(--accent); font-style: normal; background: var(--accent-soft); padding: 0 4px; border-radius: 3px; }
.pylon-landing .policy-art .pcard .badge { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; padding: 3px 7px; border-radius: 999px; background: var(--pos-soft); color: var(--pos); font-weight: 500; }

/* === PRIMITIVES GRID === */
.pylon-landing .prims { display: grid; grid-template-columns: repeat(3, 1fr); gap: 0; margin-top: 72px; border-top: 1px solid var(--line); border-left: 1px solid var(--line); }
.pylon-landing .prim {
  padding: 28px 28px 30px;
  border-right: 1px solid var(--line);
  border-bottom: 1px solid var(--line);
  background: var(--bg-card);
  position: relative;
  transition: background .15s ease;
}
/* Hover bg uses --bg-alt so it follows whichever theme is in
   effect. Was hardcoded #fdfcfa (warm cream) — a stale leftover
   from light mode that flashed bright on the dark page. */
.pylon-landing .prim:hover { background: var(--bg-alt); }
.pylon-landing .prim .top { display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 14px; }
.pylon-landing .prim .icon {
  width: 36px; height: 36px; border-radius: 8px;
  background: var(--bg-alt); border: 1px solid var(--line-2);
  display: flex; align-items: center; justify-content: center;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 13px; color: var(--text);
  font-weight: 500;
}
.pylon-landing .prim .tag { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 10.5px; color: var(--text-3); letter-spacing: .08em; text-transform: uppercase; padding: 3px 8px; border-radius: 999px; background: var(--bg-alt); }
.pylon-landing .prim .tag.game { color: var(--accent); background: var(--accent-soft); }
.pylon-landing .prim .tag.web { color: var(--text-2); background: var(--bg-card); box-shadow: inset 0 0 0 1px var(--line); }
.pylon-landing .prim h4 { font-size: 18px; letter-spacing: -.015em; font-weight: 600; margin: 0 0 6px; }
.pylon-landing .prim p { font-size: 13.5px; color: var(--text-2); line-height: 1.5; margin: 0; }
.pylon-landing .prim p code { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; background: var(--bg-alt); padding: 1px 5px; border-radius: 3px; color: var(--text); border: 1px solid var(--line); }

/* === DEPLOY LANES === */
.pylon-landing .lanes { margin-top: 64px; display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); border: 1px solid var(--line); border-radius: 14px; overflow: hidden; background: var(--bg-card); }
.pylon-landing .lane { padding: 28px 24px; border-right: 1px solid var(--line); display: flex; flex-direction: column; gap: 10px; position: relative; transition: background .2s ease; }
.pylon-landing .lane:last-child { border-right: none; }
/* Same theme-fix as .prim:hover above. */
.pylon-landing .lane:hover { background: var(--bg-alt); }
.pylon-landing .lane .top { display: flex; justify-content: space-between; align-items: center; font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; color: var(--text-3); letter-spacing: .08em; text-transform: uppercase; }
.pylon-landing .lane .top .num { color: var(--accent); font-weight: 500; }
.pylon-landing .lane h4 { font-size: 22px; font-weight: 600; letter-spacing: -.02em; margin: 6px 0 4px; }
.pylon-landing .lane .cmd { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12.5px; background: var(--code-bg); color: var(--code-green); padding: 7px 11px; border-radius: 6px; align-self: flex-start; margin: 4px 0; }
.pylon-landing .lane p { font-size: 13.5px; color: var(--text-2); line-height: 1.5; margin: 0; }
.pylon-landing .lane .footer-line { margin-top: auto; padding-top: 18px; border-top: 1px dashed var(--line); font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; color: var(--text-3); display: flex; justify-content: space-between; align-items: center; letter-spacing: .04em; }

/* === COMPARE === */
.pylon-landing .compare-wrap { margin-top: 64px; }
.pylon-landing .compare { border: 1px solid var(--line); border-radius: 14px; overflow: hidden; background: var(--bg-card); }
.pylon-landing .compare table { width: 100%; border-collapse: collapse; }
.pylon-landing .compare th, .pylon-landing .compare td { padding: 16px 20px; text-align: left; font-size: 14px; border-bottom: 1px solid var(--line); }
.pylon-landing .compare thead th { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; color: var(--text-3); text-transform: uppercase; letter-spacing: .08em; font-weight: 500; background: var(--bg-alt); }
.pylon-landing .compare th.us, .pylon-landing .compare td.us { background: linear-gradient(180deg, #faf8ff, #f1edff); border-left: 1px solid var(--accent); border-right: 1px solid var(--accent); }
.pylon-landing .compare thead th.us { color: var(--accent-deep); font-weight: 600; border-top: 2px solid var(--accent); }
.pylon-landing .compare tbody tr:last-child td { border-bottom: none; }
.pylon-landing .compare td.label { color: var(--ink); font-weight: 500; }
.pylon-landing .compare td .ind { display: inline-flex; align-items: center; gap: 6px; font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; color: var(--text-2); }
.pylon-landing .dot-yes { width: 8px; height: 8px; border-radius: 50%; background: var(--pos); }
.pylon-landing .dot-part { width: 8px; height: 8px; border-radius: 50%; background: linear-gradient(90deg, var(--pos) 50%, var(--line) 50%); border: 1px solid var(--line-2); }
.pylon-landing .dot-no { width: 8px; height: 8px; border-radius: 50%; background: transparent; border: 1px solid var(--line-2); }

/* === BIG QUOTE / PHILOSOPHY === */
.pylon-landing .philo { padding: 144px 0; border-bottom: 1px solid var(--line); background: linear-gradient(180deg, var(--bg), var(--bg-alt)); }
.pylon-landing .philo .quote { font-size: clamp(28px, 3vw, 42px); line-height: 1.14; letter-spacing: -.025em; font-weight: 500; max-width: 18ch; color: var(--ink); }
.pylon-landing .philo .who { margin-top: 36px; display: flex; align-items: center; gap: 14px; font-size: 14px; color: var(--text-2); }
.pylon-landing .philo .who .ava { width: 40px; height: 40px; border-radius: 50%; background: var(--ink); color: #fff; display: inline-flex; align-items: center; justify-content: center; font-weight: 600; letter-spacing: -.01em; }
.pylon-landing .philo .who b { color: var(--ink); font-weight: 500; }

/* === QUICKSTART === */
.pylon-landing .qs-wrap { margin-top: 56px; display: grid; grid-template-columns: 1.05fr 1fr; gap: 28px; align-items: stretch; }
.pylon-landing .qs-term {
  background: var(--code-bg); color: var(--code-text);
  border-radius: 14px; overflow: hidden; border: 1px solid #1c1c22;
  font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 13px;
  position: relative;
  box-shadow: var(--shadow-md);
}
.pylon-landing .qs-term .head { padding: 12px 16px; border-bottom: 1px solid #1c1c22; display: flex; align-items: center; gap: 10px; background: linear-gradient(180deg, #16161b, #0c0c10); font-size: 11px; color: var(--code-mute); letter-spacing: .06em; text-transform: uppercase; }
.pylon-landing .qs-term .head .dots { display: flex; gap: 6px; }
.pylon-landing .qs-term .head .dots i { width: 11px; height: 11px; border-radius: 50%; background: #2a2a32; display: inline-block; }
.pylon-landing .qs-term .body { padding: 18px 20px 24px; line-height: 1.7; }
.pylon-landing .qs-term .pr { color: var(--code-orange); }
.pylon-landing .qs-term .ok { color: var(--code-green); }
.pylon-landing .qs-term .info { color: var(--code-blue); }
.pylon-landing .qs-term .dim { color: var(--code-mute); }
.pylon-landing .qs-term .blink { display: inline-block; width: 7px; height: 14px; background: var(--code-text); vertical-align: middle; margin-left: 2px; animation: pylon-blink 1s step-end infinite; }
@keyframes pylon-blink { 50% { opacity: 0; } }

.pylon-landing .qs-steps { display: flex; flex-direction: column; gap: 14px; }
.pylon-landing .qs-step { border: 1px solid var(--line); border-radius: 12px; padding: 18px 22px; background: var(--bg-card); display: grid; grid-template-columns: 28px 1fr; gap: 14px; align-items: flex-start; }
.pylon-landing .qs-step .n { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; color: var(--accent); margin-top: 2px; }
.pylon-landing .qs-step h5 { margin: 0 0 6px; font-size: 16px; font-weight: 600; letter-spacing: -.015em; }
.pylon-landing .qs-step p { margin: 0; font-size: 13.5px; color: var(--text-2); line-height: 1.5; }
.pylon-landing .qs-step p code { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 12px; background: var(--bg-alt); padding: 1px 5px; border-radius: 3px; color: var(--text); border: 1px solid var(--line); }

/* === BIG CTA === */
/* In dark mode this is a deeper well rather than the inverse
   surface it was in light mode — page bg is #0a0a0c, the CTA
   block drops one notch to #050507 so it reads as the final
   "step down" before the footer. The accent-glow + grid mask
   keep the section visually distinct without needing a color
   inversion. */
.pylon-landing .cta-block { position: relative; padding: 64px 0 80px; text-align: left; overflow: hidden; background: #050507; color: #f3f3f4; border-top: 1px solid var(--line); }
.pylon-landing .cta-block .bg-grid {
  position: absolute; inset: 0;
  background-image:
    linear-gradient(rgba(255,255,255,.03) 1px, transparent 1px),
    linear-gradient(90deg, rgba(255,255,255,.03) 1px, transparent 1px);
  background-size: 56px 56px;
  mask-image: radial-gradient(900px 500px at 80% 50%, #000 30%, transparent 75%);
  -webkit-mask-image: radial-gradient(900px 500px at 80% 50%, #000 30%, transparent 75%);
}
.pylon-landing .cta-block .accent-glow { position: absolute; right: -200px; top: -100px; width: 600px; height: 600px; background: radial-gradient(circle, rgba(139,92,246,.18), transparent 60%); pointer-events: none; }
.pylon-landing .cta-block .inner { position: relative; max-width: 900px; }
.pylon-landing .cta-block .eyebrow { color: var(--accent); }
.pylon-landing .cta-block .eyebrow::before { background: var(--accent); }
.pylon-landing .cta-block h2 { font-size: clamp(36px, 4.2vw, 56px); letter-spacing: -.035em; line-height: 1.02; font-weight: 600; margin: 12px 0; color: #fff; }
.pylon-landing .cta-block p { font-size: 18px; color: rgba(255,255,255,.65); max-width: 560px; line-height: 1.5; margin: 0 0 32px; }
.pylon-landing .cta-block .ctas { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
.pylon-landing .cta-block .ctas .term-pill { background: rgba(255,255,255,.04); border-color: rgba(255,255,255,.1); color: #f3f3f4; }
.pylon-landing .cta-block .ctas .term-pill .prompt { color: rgba(255,255,255,.5); }
.pylon-landing .cta-block .ctas .term-pill .copy { background: rgba(255,255,255,.08); color: rgba(255,255,255,.7); }

/* === FOOTER === */
.pylon-landing footer { padding: 64px 0 40px; background: var(--bg); }
.pylon-landing .foot-grid { display: grid; grid-template-columns: 1.6fr 1fr 1fr 1fr; gap: 40px; }
.pylon-landing .foot-grid .brand-col { padding-right: 40px; }
.pylon-landing .foot-grid .brand-col p { font-size: 13.5px; color: var(--text-2); line-height: 1.55; margin: 12px 0 0; max-width: 320px; }
.pylon-landing .foot-grid h6 { font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11px; letter-spacing: .12em; text-transform: uppercase; color: var(--text-3); margin: 0 0 14px; font-weight: 500; }
.pylon-landing .foot-grid ul { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 9px; }
.pylon-landing .foot-grid li { font-size: 13.5px; }
.pylon-landing .foot-grid li a { color: var(--text-2); transition: color .15s ease; }
.pylon-landing .foot-grid li a:hover { color: var(--accent); }
.pylon-landing .foot-meta { margin-top: 56px; padding-top: 22px; border-top: 1px solid var(--line); display: flex; justify-content: space-between; align-items: center; font-family: var(--font-geist-mono), ui-monospace, monospace; font-size: 11.5px; color: var(--text-3); }
.pylon-landing .foot-meta .status { display: inline-flex; align-items: center; gap: 8px; }
.pylon-landing .foot-meta .status::before { content: ""; width: 7px; height: 7px; border-radius: 50%; background: var(--pos); box-shadow: 0 0 0 3px var(--pos-soft); }

@media (max-width: 1100px) {
  .pylon-landing .product-body { grid-template-columns: 200px 1fr; }
  .pylon-landing .app-aside { display: none; }
  .pylon-landing .feature-row, .pylon-landing .feature-row.flip { grid-template-columns: 1fr; gap: 40px; }
  .pylon-landing .feature-row.flip .feature-copy { order: 1; }
  .pylon-landing .feature-row.flip .feature-art { order: 2; }
  .pylon-landing .prims { grid-template-columns: 1fr 1fr; }
  .pylon-landing .lanes { grid-template-columns: 1fr 1fr; }
  .pylon-landing .qs-wrap { grid-template-columns: 1fr; }
  .pylon-landing .foot-grid { grid-template-columns: 1fr 1fr; }
  .pylon-landing .nav-links { display: none; }
  .pylon-landing .nav-burger { display: inline-flex; }
  /* Hero stacks to one column below ~1100px so the product mock has
     enough room to render its sidebar + main + code column. */
  .pylon-landing .hero-layout {
    grid-template-columns: 1fr;
    gap: 48px;
    padding-bottom: 0;
  }
  .pylon-landing .hero-layout .product-frame { margin-top: 0; }
  .pylon-landing .logos .row { grid-template-columns: repeat(3, 1fr); }
}

@media (max-width: 720px) {
  .pylon-landing .shell,
  .pylon-landing .shell-wide {
    padding: 0 20px;
  }

  .pylon-landing .nav-inner {
    gap: 12px;
    padding: 12px 0;
  }

  .pylon-landing .brand {
    flex-shrink: 0;
  }

  .pylon-landing .nav-cta {
    min-width: 0;
  }

  .pylon-landing .nav-cta .btn.ghost,
  .pylon-landing .nav-cta .nav-github {
    display: none;
  }

  .pylon-landing .nav-cta .btn {
    white-space: nowrap;
    padding: 8px 12px;
    font-size: 13px;
  }

  .pylon-landing .hero {
    padding-top: 56px;
  }

  .pylon-landing h1.h1 {
    font-size: 44px;
    line-height: 1.02;
    max-width: 11ch;
    margin: 24px 0 20px;
  }

  .pylon-landing .hero p.lede {
    font-size: 17px;
    line-height: 1.52;
    margin-bottom: 28px;
  }

  .pylon-landing .hero-ctas {
    align-items: stretch;
    flex-direction: column;
  }

  .pylon-landing .hero-ctas .btn,
  .pylon-landing .hero-ctas .term-pill,
  .pylon-landing .cta-block .ctas .btn,
  .pylon-landing .cta-block .ctas .term-pill {
    width: 100%;
    justify-content: center;
    min-height: 42px;
  }

  .pylon-landing .term-pill {
    padding-right: 10px;
  }

  .pylon-landing .product-frame {
    margin-top: 8px;
    border-radius: 12px;
  }

  .pylon-landing .product-chrome {
    gap: 10px;
    padding: 10px 12px;
  }

  .pylon-landing .product-chrome .dots {
    gap: 5px;
  }

  .pylon-landing .product-chrome .dots i {
    width: 9px;
    height: 9px;
  }

  .pylon-landing .product-chrome .url {
    min-width: 0;
    max-width: none;
    padding: 0 10px;
  }

  .pylon-landing .product-chrome .url span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pylon-landing .product-chrome .right {
    display: none;
  }

  .pylon-landing .product-body {
    display: block;
    min-height: auto;
  }

  .pylon-landing .app-side,
  .pylon-landing .app-aside {
    display: none;
  }

  .pylon-landing .app-main {
    padding: 16px;
  }

  .pylon-landing .app-main .crumb {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pylon-landing .app-main h2.app-title {
    align-items: flex-start;
    flex-direction: column;
    gap: 8px;
    font-size: 19px;
    margin-bottom: 14px;
  }

  .pylon-landing .metrics {
    grid-template-columns: 1fr;
    gap: 10px;
  }

  .pylon-landing .metric {
    min-height: 96px;
    padding-right: 76px;
  }

  .pylon-landing .metric .num {
    font-size: 24px;
  }

  .pylon-landing .metric .spark {
    width: 52px;
    opacity: .75;
  }

  .pylon-landing .tbl-head {
    display: none;
  }

  .pylon-landing .tbl-row {
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 6px 12px;
    padding: 13px 14px;
  }

  .pylon-landing .tbl-row .name {
    grid-column: 1;
    grid-row: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pylon-landing .tbl-row > :nth-child(2) {
    grid-column: 1 / -1;
    grid-row: 2;
  }

  .pylon-landing .tbl-row > :nth-child(3) {
    grid-column: 2;
    grid-row: 1;
    font-weight: 500;
    text-align: right;
  }

  .pylon-landing .tbl-row > :nth-child(4) {
    grid-column: 1;
    grid-row: 3;
  }

  .pylon-landing .tbl-row > :nth-child(5) {
    grid-column: 2;
    grid-row: 3;
    text-align: right;
  }

  .pylon-landing .logos {
    margin-top: 56px;
    padding: 28px 0;
  }

  .pylon-landing .logos .row {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 24px 16px;
  }

  .pylon-landing section.block {
    padding: 76px 0;
  }

  .pylon-landing h2.h2 {
    font-size: 36px;
    line-height: 1.08;
  }

  .pylon-landing .section-lede {
    font-size: 16px;
    margin-bottom: 0;
  }

  .pylon-landing .feature-row,
  .pylon-landing .feature-row.flip {
    gap: 30px;
    margin-top: 52px;
  }

  .pylon-landing .feature-copy h3 {
    font-size: 26px;
  }

  .pylon-landing .feature-copy p {
    font-size: 15px;
  }

  .pylon-landing .feature-bullets li {
    font-size: 14px;
  }

  .pylon-landing .schema-art {
    overflow-x: auto;
  }

  .pylon-landing .schema-tabs,
  .pylon-landing .schema-body {
    min-width: 540px;
  }

  .pylon-landing .schema-tabs .tab {
    flex: 0 0 auto;
  }

  .pylon-landing .schema-body {
    min-height: 320px;
  }

  .pylon-landing .live-art,
  .pylon-landing .policy-art {
    padding: 16px;
  }

  .pylon-landing .live-art .row-set .r {
    grid-template-columns: 68px minmax(0, 1fr) auto;
    gap: 8px;
    padding: 7px 8px;
    font-size: 11px;
  }

  .pylon-landing .live-art .row-set .r .lbl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pylon-landing .policy-art .pcard {
    grid-template-columns: 22px minmax(0, 1fr);
    align-items: flex-start;
  }

  .pylon-landing .policy-art .pcard .badge {
    grid-column: 2;
    justify-self: start;
  }

  .pylon-landing .policy-art .pcard .rule {
    overflow-wrap: anywhere;
  }

  .pylon-landing .prims,
  .pylon-landing .lanes,
  .pylon-landing .qs-wrap,
  .pylon-landing .foot-grid {
    grid-template-columns: 1fr;
  }

  .pylon-landing .prims {
    margin-top: 48px;
  }

  .pylon-landing .prim,
  .pylon-landing .lane {
    padding: 22px;
  }

  .pylon-landing .lane {
    border-right: none;
    border-bottom: 1px solid var(--line);
  }

  .pylon-landing .lane:last-child {
    border-bottom: none;
  }

  .pylon-landing .lane .top,
  .pylon-landing .lane .footer-line {
    gap: 12px;
    flex-wrap: wrap;
  }

  .pylon-landing .lane .cmd {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pylon-landing .compare {
    overflow-x: auto;
  }

  .pylon-landing .compare table {
    min-width: 660px;
  }

  .pylon-landing .compare th,
  .pylon-landing .compare td {
    padding: 14px 16px;
  }

  .pylon-landing .qs-term .body {
    overflow-x: auto;
    padding: 16px;
  }

  .pylon-landing .qs-term .body > div {
    white-space: nowrap;
  }

  .pylon-landing .qs-step {
    grid-template-columns: 24px minmax(0, 1fr);
    padding: 16px;
  }

  .pylon-landing .cta-block {
    padding: 76px 0 84px;
  }

  .pylon-landing .cta-block h2 {
    font-size: 40px;
    line-height: 1.04;
  }

  .pylon-landing .cta-block p {
    font-size: 16px;
  }

  .pylon-landing .cta-block .ctas {
    align-items: stretch;
    flex-direction: column;
  }

  .pylon-landing footer {
    padding: 48px 0 32px;
  }

  .pylon-landing .foot-grid {
    gap: 28px;
  }

  .pylon-landing .foot-grid .brand-col {
    padding-right: 0;
  }

  .pylon-landing .foot-meta {
    align-items: flex-start;
    flex-direction: column;
    gap: 12px;
    margin-top: 20px;
  }
}

@media (max-width: 420px) {
  .pylon-landing .shell,
  .pylon-landing .shell-wide {
    padding: 0 16px;
  }

  .pylon-landing .nav-cta .btn.dark {
    padding: 8px 10px;
  }

  .pylon-landing h1.h1 {
    font-size: 40px;
  }

  .pylon-landing h2.h2 {
    font-size: 32px;
  }

  .pylon-landing .cta-block h2 {
    font-size: 36px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .pylon-landing *,
  .pylon-landing *::before,
  .pylon-landing *::after {
    animation-duration: .01ms !important;
    animation-iteration-count: 1 !important;
    scroll-behavior: auto !important;
    transition-duration: .01ms !important;
  }
}
`;

/// Languages the H1 cycles through, paired with a brand-tinted
/// color scheme for the rotating pill. Soft tints (not the full
/// brand color) so the H1 still reads as "pill highlight" not
/// "loud ad". Each entry sets:
///   bg — the pill's fill, ~7% saturation of the brand color
///   fg — the text + border, the punchy brand value
///
/// Order matters — `TypeScript` is the SSR default (most landing
/// visitors expect TS first), then we rotate through the other
/// platforms Pylon ships first-class SDKs for.
const LANGS: ReadonlyArray<{ name: string; bg: string; fg: string }> = [
	// Pill colors for dark-mode bg (#0a0a0c). Pattern: each pill's
	// `bg` is a low-alpha tint of the brand hue (reads as soft
	// glow on the canvas), `fg` is a high-luminance step on the
	// same hue so the text remains crisp.
	// TypeScript — TS blue
	{ name: "TypeScript", bg: "rgba(49,120,198,.18)", fg: "#7eb6ff" },
	// Swift — Apple orange
	{ name: "Swift", bg: "rgba(240,81,56,.20)", fg: "#ff9b78" },
	// React — atom cyan
	{ name: "React", bg: "rgba(97,218,251,.16)", fg: "#7adcf0" },
	// React Native — purple-blue, distinct from React proper
	{ name: "React Native", bg: "rgba(167,139,250,.20)", fg: "#cdbafe" },
	// Next.js — graphite; soft-white pill so it doesn't read as
	// a CTA button against the dark canvas.
	{ name: "Next.js", bg: "rgba(255,255,255,.10)", fg: "#fafafa" },
] as const;

/// "React Native" is the longest entry — used as the ghost
/// element that pins the pill's width so shorter languages don't
/// make the surrounding text ("apps.") visibly jump. Kept as a
/// derived constant so the source of truth stays LANGS above.
const LONGEST_LANG = LANGS.reduce((a, b) =>
	b.name.length > a.name.length ? b : a,
).name;

const TABLE_ROWS = [
	{ initials: "JM", name: "Jordan Moss", email: "jordan@kindly.io", total: "$89.00", status: "paid" as const, age: "3s ago" },
	{ initials: "RP", name: "Rhea Patel", email: "rhea@northbeam.co", total: "$145.00", status: "paid" as const, age: "22s ago" },
	{ initials: "MT", name: "Maya Torres", email: "maya@plant.studio", total: "$22.50", status: "pending" as const, age: "1m ago" },
	{ initials: "AC", name: "Alex Chen", email: "alex@vellum.dev", total: "$312.00", status: "paid" as const, age: "4m ago" },
	{ initials: "NS", name: "Noor Saleh", email: "noor@kits.co", total: "$56.00", status: "failed" as const, age: "6m ago" },
];

const EVENT_TPL: Array<[string, string]> = [
	["db.insert <em>Order</em>", "+1"],
	["policy.check", "ok"],
	["stream.diff", "→4"],
	["workflow.step <em>charge</em>", "ok"],
	["db.update <em>Order</em>", "↻"],
	["auth.session", "ok"],
	["search.index <em>Order</em>", "ok"],
	["cron.tick <em>cleanup</em>", "ok"],
];

const EVENT_TIMES = ["now", "3s", "12s", "24s", "48s"];

const PRIMITIVES: Array<{ icon: string; tag: "web" | "app" | "game"; title: string; body: React.ReactNode }> = [
	{ icon: "⧉", tag: "web", title: "Server-rendered React", body: <>Streaming SSR with hydration and per-route code splitting. Pages render on the server and ship only the JS the route needs.</> },
	{ icon: "↳", tag: "web", title: "<Link> & <Image>", body: <>Client-side navigation with prefetch, and image optimization in Rust — resize, WebP, content-addressed cache. No <code>next/image</code>.</> },
	{ icon: "~", tag: "web", title: "Tailwind, wired", body: <>Tailwind compiles on save and ships with the page. No second build step, no PostCSS config to babysit.</> },
	{ icon: "{ }", tag: "app", title: "Typed schema", body: <>Entities with composite indexes and relations. Migrations apply on save, generates a typed client.</> },
	{ icon: "⇄", tag: "app", title: "Live queries", body: <><code>db.useQuery</code> is a WebSocket subscription. Diffs over the wire on every relevant write.</> },
	{ icon: "fn", tag: "app", title: "Server functions", body: <>Queries, mutations, actions in TypeScript with <code>v.*</code> validators. Filename is the RPC name.</> },
	{ icon: "==", tag: "app", title: "Row-level policies", body: <>Access rules next to the schema. Compiled to bytecode, evaluated in the query plan.</> },
	{ icon: "⌧", tag: "app", title: "Auth, included", body: <>Magic-link, 25+ OAuth providers, OIDC discovery, guest sessions, API keys. No third-party SDK.</> },
	{ icon: "⌕", tag: "app", title: "Faceted search", body: <>BM25 + live facets across millions of rows. Maintained in the same transaction as your writes.</> },
	{ icon: "↥", tag: "app", title: "Files & uploads", body: <>Presigned uploads to local disk or any S3-compatible bucket. R2, Backblaze, MinIO — one env var.</> },
	{ icon: "↺", tag: "app", title: "Durable workflows", body: <>Multi-step flows with sleep, retries, event waits. State checkpointed on every step.</> },
	{ icon: "⏱", tag: "app", title: "Jobs & cron", body: <>Enqueue with <code>ctx.schedule</code>. Cron lives in the manifest — version-controlled with code.</> },
	{ icon: "db", tag: "app", title: "SQLite or Postgres", body: <>SQLite is the default. Set <code>DATABASE_URL</code> and the same schema targets Postgres.</> },
	{ icon: "◉", tag: "game", title: "Rooms & presence", body: <>WebSocket rooms, per-member presence, join/leave events, broadcast. No Ably, no Pusher.</> },
	{ icon: "⟳", tag: "game", title: "Tick-based shards", body: <>Authoritative 20/30/60 tps loops in Rust. Area-of-interest, snapshot + delta replication, late-join.</> },
];

/**
 * Compact star count for the GitHub nav badge. Under 1k shows the
 * exact number; over 1k rounds to one-decimal "k" (e.g. 2400 → 2.4k).
 * Mirrors the format GitHub itself uses on the repo page.
 */
function formatStars(n: number): string {
	if (n < 1000) return n.toLocaleString();
	if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k`;
	return `${Math.round(n / 1000)}k`;
}

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

export function LandingPage({
	version,
	lastShippedISO,
	stars,
}: {
	version: string;
	lastShippedISO: string | null;
	stars: number | null;
}) {
	const [revenue, setRevenue] = useState(48920);
	const [orderCount, setOrderCount] = useState(1284);
	const [clientCount, setClientCount] = useState(47);
	const [flashIdx, setFlashIdx] = useState<number | null>(null);
	const [menuOpen, setMenuOpen] = useState(false);
	// Rotating language slot in the H1. Server-renders the first
	// entry so initial paint shows "TypeScript apps." (the most
	// common landing intent), then cycles client-side.
	const [langIdx, setLangIdx] = useState(0);
	const [events, setEvents] = useState<Array<{ k: string; v: string }>>([
		{ k: "db.insert <em>Order</em>", v: "+1" },
		{ k: "policy.check", v: "ok" },
		{ k: "db.insert <em>Order</em>", v: "+1" },
		{ k: "stream.diff", v: "→4" },
		{ k: "auth.session", v: "ok" },
	]);

	useEffect(() => {
		const tickMetrics = setInterval(() => {
			setRevenue((r) => r + Math.floor(Math.random() * 80) + 10);
			setOrderCount((o) => o + (Math.random() > 0.6 ? 1 : 0));
			setClientCount((c) => Math.max(38, Math.min(62, c + (Math.random() > 0.5 ? 1 : -1))));
		}, 1400);
		const tickFlash = setInterval(() => {
			setFlashIdx(Math.floor(Math.random() * TABLE_ROWS.length));
		}, 2200);
		const tickEvents = setInterval(() => {
			const [k, v] = EVENT_TPL[Math.floor(Math.random() * EVENT_TPL.length)]!;
			setEvents((prev) => [{ k, v }, ...prev].slice(0, 5));
		}, 3200);
		// H1 language rotation. Slower cadence than the metrics so
		// the heading doesn't compete with the product mock for
		// attention.
		const tickLang = setInterval(() => {
			setLangIdx((i) => (i + 1) % LANGS.length);
		}, 2200);
		return () => {
			clearInterval(tickMetrics);
			clearInterval(tickFlash);
			clearInterval(tickEvents);
			clearInterval(tickLang);
		};
	}, []);

	// Live "last shipped" label for the hero proof badge. Today / N days
	// ago / a calendar date — pick the form that reads as alive without
	// stretching the truth (week-old releases shouldn't claim "today").
	const shippedLabel = (() => {
		if (!lastShippedISO) return null;
		const shippedAt = new Date(`${lastShippedISO}T00:00:00Z`);
		const now = new Date();
		const days = Math.floor(
			(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()) -
				shippedAt.getTime()) /
				86_400_000,
		);
		if (days <= 0) return "shipped today";
		if (days === 1) return "shipped yesterday";
		if (days < 14) return `shipped ${days}d ago`;
		return `shipped ${shippedAt.toLocaleDateString("en-US", { month: "short", day: "numeric" })}`;
	})();

	return (
		<>
			<style dangerouslySetInnerHTML={{ __html: DESIGN_CSS }} />
			<div className="pylon-landing">
				{/* NAV */}
				<nav className={`nav${menuOpen ? " menu-open" : ""}`}>
					<div className="shell nav-inner">
						<Link className="brand" href="/" onClick={() => setMenuOpen(false)}>
							<PylonMark size={20} style={{ color: "var(--ink)" }} />
							Pylon
						</Link>
						<ul className="nav-links">
							<li><a href="#features">Product</a></li>
							<li><a href="#primitives">Primitives</a></li>
							<li><a href="https://docs.pylonsync.com">Docs</a></li>
							<li><a href="https://github.com/pylonsync/pylon/releases">Changelog</a></li>
							<li><a href="#compare">Compare</a></li>
						</ul>
						<div className="nav-cta">
							<a
								className="nav-github"
								href="https://github.com/pylonsync/pylon"
								target="_blank"
								rel="noopener noreferrer"
								aria-label="Pylon on GitHub"
							>
								<svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
									<path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
								</svg>
								{stars !== null ? (
									<span className="nav-github-stars">★ {formatStars(stars)}</span>
								) : (
									<span className="nav-github-stars">GitHub</span>
								)}
							</a>
							<Link className="btn ghost" href="https://cloud.pylonsync.com/login">Sign in</Link>
							<Link className="btn dark" href="https://cloud.pylonsync.com">Start building →</Link>
							<button
								type="button"
								className="nav-burger"
								aria-label={menuOpen ? "Close menu" : "Open menu"}
								aria-expanded={menuOpen}
								onClick={() => setMenuOpen((v) => !v)}
							>
								<span /><span /><span />
							</button>
						</div>
					</div>
					<div className="nav-sheet" aria-hidden={!menuOpen}>
						<a href="#features" onClick={() => setMenuOpen(false)}>Product</a>
						<a href="#primitives" onClick={() => setMenuOpen(false)}>Primitives</a>
						<a href="https://docs.pylonsync.com">Docs</a>
						<a href="https://github.com/pylonsync/pylon/releases">Changelog</a>
						<a href="#compare" onClick={() => setMenuOpen(false)}>Compare</a>
						<a href="https://cloud.pylonsync.com/login" className="sheet-signin">Sign in →</a>
					</div>
				</nav>

				{/* HERO */}
				<header className="hero">
					<div className="hero-grid-bg" />
					<div className="shell hero-layout">
						<div className="hero-text">
							{/* Ship badge promoted to eyebrow position. Standard hero pattern
							    (Linear, Vercel) — the "this is alive" proof catches the eye
							    above the H1, then leads into the value prop. Frees the area
							    below the CTAs from competing rows. */}
							{shippedLabel && (
								<a
									className="hero-ship-badge hero-ship-badge--eyebrow"
									href="https://github.com/pylonsync/pylon/releases"
									target="_blank"
									rel="noopener noreferrer"
								>
									<span className="dot" />
									<span className="ver">v{version}</span>
									<span className="sep">·</span>
									<span className="when">{shippedLabel}</span>
									<span className="arrow">→</span>
								</a>
							)}
							<h1 className="h1">
								The full-stack realtime framework for{" "}
								<span
									className="lang-rotator"
									style={{
										// Inline because each tick varies — kept on the
										// host so the chrome itself (bg + border) cross-fades
										// when the language changes, separately from the
										// word fade-up below. CSS transition on the pill
										// smooths the color swap. Border color reuses the
										// pill's bg (which is already low-alpha brand tint),
										// giving a 1px halo without needing to derive
										// alpha math from the fg.
										background: LANGS[langIdx]!.bg,
										color: LANGS[langIdx]!.fg,
										borderColor: LANGS[langIdx]!.bg,
									}}
								>
									{/* Ghost: claims the pill's width based on the
									    longest entry. Always present, always invisible.
									    Without this, the pill would resize per word and
									    "apps." would visibly slide left/right. */}
									<span aria-hidden="true" className="lang-ghost">
										{LONGEST_LANG}
									</span>
									{/* Visible word: stacked over the ghost in the same
									    grid cell. `key={langIdx}` makes React replace
									    the node on each tick, re-running the entry
									    animation. */}
									<span key={langIdx} className="lang-word">
										{LANGS[langIdx]!.name}
									</span>
								</span>{" "}
								apps.
							</h1>
							<p className="lede">
								Render your React frontend and run your backend from <b>one server.</b>{" "}
								Server-rendered React, routing, and image optimization next to schema, live queries, auth, jobs, and search.{" "}
								SQLite or Postgres. Deploy to your VPS or Pylon Cloud. <b>No separate Next.js.</b>
							</p>
							{/* Primary CTA: actually start building. The fastest
							    path from "landed on the site" to "running app on
							    localhost" is `npm create`. Promoted from the prior
							    "secondary" slot — the old hero pushed "Start free
							    on Pylon Cloud" first, which was the wrong frame:
							    visitors here are evaluating the framework, not the
							    managed product. Cloud signup still lives in the
							    final CTA section + nav. */}
							<div className="hero-ctas">
								<CopyCommand command="npm create @pylonsync/pylon@latest" />
								<a className="btn ghost" href="https://docs.pylonsync.com">Read the docs →</a>
							</div>
						</div>

						{/* PRODUCT MOCK */}
						<div className="product-frame">
							<div className="product-chrome">
								<div className="dots"><i /><i /><i /></div>
								<div className="url"><span>yourapp.com/acme/orders</span></div>
								<div className="right">
									<span className="badge">live · {clientCount} connected</span>
									<span className="kbd">⌘</span><span style={{ color: "var(--text-3)", fontSize: 12 }}>K</span>
								</div>
							</div>
							<div className="product-body">
								<aside className="app-side">
									<div className="group">
										<h6>Workspace</h6>
										<ul><li><span className="glyph" />Acme · production<span className="count">prod</span></li></ul>
									</div>
									<div className="group">
										<h6>Studio</h6>
										<ul>
											<li><span className="glyph" />Overview</li>
											<li className="active"><span className="glyph" />Orders<span className="count">1.2k</span></li>
											<li><span className="glyph" />Customers<span className="count">487</span></li>
											<li><span className="glyph" />Sessions<span className="count">2.1k</span></li>
											<li><span className="glyph" />Files<span className="count">312</span></li>
										</ul>
									</div>
									<div className="group">
										<h6>Backend</h6>
										<ul>
											<li><span className="glyph" />Functions</li>
											<li><span className="glyph" />Live queries</li>
											<li><span className="glyph" />Workflows</li>
											<li><span className="glyph" />Jobs &amp; cron</li>
											<li><span className="glyph" />Logs</li>
										</ul>
									</div>
									<div className="group">
										<h6>Settings</h6>
										<ul>
											<li><span className="glyph" />Auth</li>
											<li><span className="glyph" />Plugins</li>
										</ul>
									</div>
								</aside>

								<main className="app-main">
									<div className="crumb mono">acme · <b>Orders</b> · live</div>
									<h2 className="app-title">Orders dashboard <span className="live">streaming</span></h2>

									<div className="metrics">
										<div className="metric">
											<div className="label">Revenue · 24h</div>
											<div className="num">${revenue.toLocaleString()}</div>
											<div className="delta">↑ 12.4% from yesterday</div>
											<svg className="spark" viewBox="0 0 64 22" preserveAspectRatio="none">
												<polyline fill="none" stroke="#16a34a" strokeWidth="1.5" points="0,18 8,15 16,16 24,11 32,12 40,7 48,9 56,4 64,5" />
											</svg>
										</div>
										<div className="metric">
											<div className="label">Orders · 24h</div>
											<div className="num">{orderCount.toLocaleString()}</div>
											<div className="delta">↑ 8.1%</div>
											<svg className="spark" viewBox="0 0 64 22" preserveAspectRatio="none">
												<polyline fill="none" stroke="#8B5CF6" strokeWidth="1.5" points="0,16 8,14 16,12 24,15 32,10 40,12 48,8 56,9 64,5" />
											</svg>
										</div>
										<div className="metric">
											<div className="label">Live clients</div>
											<div className="num">{clientCount}</div>
											<div className="delta" style={{ color: "var(--text-3)" }}>steady · ws</div>
											<svg className="spark" viewBox="0 0 64 22" preserveAspectRatio="none">
												<polyline fill="none" stroke="#a1a1aa" strokeWidth="1.5" points="0,12 8,11 16,13 24,10 32,12 40,11 48,13 56,10 64,12" />
											</svg>
										</div>
									</div>

									<div className="tbl">
										<div className="tbl-head">
											<div>Customer</div><div>Email</div><div>Total</div><div>Status</div><div>Created</div>
										</div>
										<div>
											{TABLE_ROWS.map((row, i) => (
												<div key={row.initials} className={`tbl-row${flashIdx === i ? " flash" : ""}`}>
													<div className="name"><span className="avatar">{row.initials}</span>{row.name}</div>
													<div className="id">{row.email}</div>
													<div>{row.total}</div>
													<div><span className={`status-pill ${row.status}`}>{row.status}</span></div>
													<div className="id">{row.age}</div>
												</div>
											))}
										</div>
									</div>
								</main>

								<aside className="app-aside">
									<div className="aside-title">Live query <span className="ws">ws · 14ms</span></div>
									<div className="code-mini">
										<div className="file"><b>Dashboard.tsx</b><span>tsx</span></div>
										<pre>
											<span className="k1">const</span>{" { "}<span className="t1">data</span>: orders {"} ="}
											{"\n  db."}<span className="f1">useQuery</span>(<span className="s1">&quot;Order&quot;</span>, {"{"}
											{"\n    where: { status: "}<span className="s1">&quot;paid&quot;</span>{" },"}
											{"\n    order: "}<span className="s1">&quot;desc&quot;</span>,
											{"\n    limit: "}<span className="n1">50</span>,
											{"\n  });"}
											{"\n\n"}
											<span className="c1">// re-runs on every write,</span>
											{"\n"}
											<span className="c1">// no polling, no invalidation</span>
										</pre>
									</div>

									<div className="events-panel">
										<div className="head"><span>Event log</span><b>last 30s</b></div>
										<div>
											{events.map((ev, i) => (
												<div key={`${ev.k}-${i}-${events.length}`} className="event">
													<span className="t">{EVENT_TIMES[i] ?? ""}</span>
													<span className="k" dangerouslySetInnerHTML={{ __html: ev.k }} />
													<span className="v">{ev.v}</span>
												</div>
											))}
										</div>
									</div>
								</aside>
							</div>
						</div>

					</div>
				</header>

				{/* FEATURES */}
				<section className="block" id="features">
					<div className="shell">
						<div className="eyebrow">The model</div>
						<h2 className="h2">Three lines of code. The whole stack.</h2>
						<p className="section-lede">
							Most stacks are a database, an API server, a pub/sub layer, and a separate frontend framework glued together. Pylon collapses it — the handler is the transaction, the query is the subscription, and the same server renders your React pages.
						</p>

						<div className="feature-row">
							<div className="feature-copy">
								<span className="tiny-tag">01 · Declare</span>
								<h3>Schema as the source of truth.</h3>
								<p>Entities and indexes in TypeScript. Migrations apply on save. Generates a fully-typed client and an OpenAPI surface, in the same step.</p>
								<ul className="feature-bullets">
									<li><span><b>Composite indexes</b> declared inline — Pylon picks the right one at query time.</span></li>
									<li><span><b>Row-level policies</b> live next to the schema, compiled to bytecode in the hot path.</span></li>
									<li><span><b>Soft delete, timestamps, slugify, versioning</b> — attach with a single line in the manifest.</span></li>
								</ul>
							</div>
							<div className="art schema-art">
								<div className="schema-tabs">
									<div className="tab on">schema.ts</div>
									<div className="tab">policies.ts</div>
									<div className="tab">manifest.json</div>
								</div>
								<div className="schema-body">
									<div className="schema-gutter">{Array.from({ length: 17 }, (_, i) => i + 1).map((n) => <div key={n}>{n}</div>)}</div>
									<div className="schema-code">
										<pre style={{ margin: 0 }}>
											<span className="k">import</span>{" { e, field, policy, timestamps, softDelete, audit } "}<span className="k">from</span>{" "}<span className="s">&quot;@pylonsync/sdk&quot;</span>;{"\n\n"}
											<span className="k">export const</span>{" "}<span className="b">Order</span>{" = e."}<span className="t">entity</span>(<span className="s">&quot;Order&quot;</span>, {"{\n"}
											{"  customer:  "}<span className="t">field</span>.id(<span className="s">&quot;Customer&quot;</span>),{"\n"}
											{"  total:     "}<span className="t">field</span>.int(),                 <span className="anno">cents</span>{"\n"}
											{"  status:    "}<span className="t">field</span>.enum([<span className="s">&quot;pending&quot;</span>,<span className="s">&quot;paid&quot;</span>,<span className="s">&quot;failed&quot;</span>]),{"\n"}
											{"  createdAt: "}<span className="t">field</span>.datetime().defaultNow(),{"\n"}
											{"})\n  .indexes(e."}<span className="t">idx</span>(<span className="s">&quot;customer&quot;</span>, <span className="s">&quot;createdAt&quot;</span>{"), e."}<span className="t">idx</span>(<span className="s">&quot;status&quot;</span>{"))\n"}
											{"  .policies("}<span className="t">policy</span>({"{\n"}
											{"    allowRead:   "}<span className="s">&quot;auth.userId == data.customer || auth.hasRole(&apos;admin&apos;)&quot;</span>,{"\n"}
											{"    allowUpdate: "}<span className="s">&quot;auth.hasRole(&apos;admin&apos;)&quot;</span>,{"\n"}
											{"  }))\n"}
											{"  .behaviors(["}<span className="t">timestamps</span>, <span className="t">softDelete</span>, <span className="t">audit</span>{"]);\n"}
											<span className="c">{"// → pylon codegen client · OpenAPI at /api/openapi"}</span>
										</pre>
									</div>
								</div>
							</div>
						</div>

						<div className="feature-row flip">
							<div className="feature-copy">
								<span className="tiny-tag">02 · Subscribe</span>
								<h3>
									<code style={{ fontFamily: "'Geist Mono',monospace", fontSize: ".86em", background: "var(--bg-alt)", padding: "2px 8px", borderRadius: 6, border: "1px solid var(--line)", fontWeight: 500 }}>
										db.useQuery
									</code>{" "}is a subscription.
								</h3>
								<p>Pylon walks the change log on every write and pushes a diff to every client whose query depends on it. No polling. No cache invalidation. No fan-out service to operate.</p>
								<ul className="feature-bullets">
									<li><span><b>WebSocket-first</b> with HTTP fallback. Reconnection, backoff and replay are handled.</span></li>
									<li><span><b>IndexedDB mirror</b> answers reads in 0.4ms while the diff is in flight.</span></li>
									<li><span><b>Optimistic mutations</b> with automatic rollback when the server rejects.</span></li>
								</ul>
							</div>
							<div className="art live-art">
								<h5><span className="pulse" />orders.live · 247 deltas / minute</h5>
								<div className="chart">
									<svg viewBox="0 0 600 160" preserveAspectRatio="none">
										<defs>
											<linearGradient id="ga" x1="0" x2="0" y1="0" y2="1">
												<stop offset="0%" stopColor="#8B5CF6" stopOpacity=".18" />
												<stop offset="100%" stopColor="#8B5CF6" stopOpacity="0" />
											</linearGradient>
										</defs>
										<path d="M0,120 L20,110 L40,118 L60,95 L80,105 L100,80 L120,90 L140,72 L160,60 L180,75 L200,55 L220,68 L240,40 L260,52 L280,38 L300,48 L320,30 L340,42 L360,28 L380,50 L400,42 L420,55 L440,38 L460,48 L480,30 L500,42 L520,28 L540,38 L560,22 L580,30 L600,18 L600,160 L0,160 Z" fill="url(#ga)" stroke="none" />
										<path d="M0,120 L20,110 L40,118 L60,95 L80,105 L100,80 L120,90 L140,72 L160,60 L180,75 L200,55 L220,68 L240,40 L260,52 L280,38 L300,48 L320,30 L340,42 L360,28 L380,50 L400,42 L420,55 L440,38 L460,48 L480,30 L500,42 L520,28 L540,38 L560,22 L580,30 L600,18" fill="none" stroke="#8B5CF6" strokeWidth="2" />
										<circle cx="600" cy="18" r="4" fill="#8B5CF6" />
										<circle cx="600" cy="18" r="8" fill="#8B5CF6" opacity=".25" />
									</svg>
								</div>
								<div className="legend"><span>−60s</span><span>−30s</span><span style={{ color: "var(--accent)" }}>now</span></div>
								<div className="row-set">
									<div className="r"><span className="id">ord_9f2a</span><span className="lbl">Jordan Moss · paid</span><span className="v">+$89</span></div>
									<div className="r"><span className="id">ord_9f2b</span><span className="lbl">Rhea Patel · paid</span><span className="v">+$145</span></div>
									<div className="r"><span className="id">ord_9f2c</span><span className="lbl">Maya Torres · pending</span><span className="v">$22.50</span></div>
								</div>
							</div>
						</div>

						<div className="feature-row">
							<div className="feature-copy">
								<span className="tiny-tag">03 · Govern</span>
								<h3>Auth and policies, where they belong.</h3>
								<p>25+ OAuth providers, magic-link, OIDC discovery, guest sessions, API keys. Row-level policies sit next to the data and compile into the query plan, so the rule runs on every read whether you remembered it or not.</p>
								<ul className="feature-bullets">
									<li><span><b>Multi-tenant</b> with <code style={{ background: "var(--bg-alt)", padding: "1px 5px", borderRadius: 3, border: "1px solid var(--line)", fontFamily: "'Geist Mono',monospace", fontSize: 12 }}>tenant_scope</code> — never write <code style={{ background: "var(--bg-alt)", padding: "1px 5px", borderRadius: 3, border: "1px solid var(--line)", fontFamily: "'Geist Mono',monospace", fontSize: 12 }}>WHERE org_id = ?</code> by hand again.</span></li>
									<li><span><b>RBAC</b> with organizations, members, roles, and per-resource overrides.</span></li>
									<li><span><b>Audit log</b> records the actor and the diff for every mutation.</span></li>
								</ul>
							</div>
							<div className="art policy-art">
								<div className="pcard">
									<span className="ic">R</span>
									<div>
										<div className="name">Order · read</div>
										<div className="rule"><em>auth.userId == data.customer</em> || <em>auth.role == &apos;admin&apos;</em></div>
									</div>
									<span className="badge">live</span>
								</div>
								<div className="pcard">
									<span className="ic">W</span>
									<div>
										<div className="name">Order · write</div>
										<div className="rule"><em>auth.role == &apos;admin&apos;</em></div>
									</div>
									<span className="badge">live</span>
								</div>
								<div className="pcard">
									<span className="ic">D</span>
									<div>
										<div className="name">Order · delete</div>
										<div className="rule">disabled · <em>soft_delete</em></div>
									</div>
									<span className="badge" style={{ background: "var(--bg-alt)", color: "var(--text-2)" }}>off</span>
								</div>
								<div className="pcard">
									<span className="ic">⌧</span>
									<div>
										<div className="name">Session · auth</div>
										<div className="rule">magic-link + <em>google</em>, <em>github</em>, <em>oidc</em></div>
									</div>
									<span className="badge">live</span>
								</div>
								<div className="pcard">
									<span className="ic">↺</span>
									<div>
										<div className="name">Order.create · workflow</div>
										<div className="rule">→ <em>charge</em> → <em>email_receipt</em> → <em>fulfill</em></div>
									</div>
									<span className="badge">live</span>
								</div>
							</div>
						</div>
					</div>
				</section>

				{/* PRIMITIVES */}
				<section className="block" id="primitives" style={{ background: "var(--bg-alt)" }}>
					<div className="shell">
						<div className="eyebrow">Fifteen primitives</div>
						<h2 className="h2">Fifteen primitives. No glue.</h2>
						<p className="section-lede">The pieces you usually stitch together ship as one system — frontend and backend. Render the React side, or use the data side alone, then layer on realtime, workflows, search, and game-shaped primitives when the product needs them.</p>

						<div className="prims">
							{PRIMITIVES.map((p) => (
								<div key={p.title} className="prim">
									<div className="top">
										<div className="icon">{p.icon}</div>
										<div className={`tag${p.tag === "game" ? " game" : ""}`}>{p.tag}</div>
									</div>
									<h4>{p.title}</h4>
									<p>{p.body}</p>
								</div>
							))}
						</div>
					</div>
				</section>

				{/* CLIENT BINDINGS */}
				<section className="block" id="clients">
					<div className="shell">
						<div className="eyebrow">Frontends</div>
						<h2 className="h2">Renders the web. Serves every other client.</h2>
						<p className="section-lede">Pylon server-renders your React app from the same process that runs your data — and the same backend feeds your SPA, mobile, and native clients with realtime subscriptions, optimistic mutations, and a typed client.</p>

						<div className="lanes">
							<div className="lane">
								<div className="top"><span><span className="num">WEB</span> · SSR</span><span>built in</span></div>
								<h4>Pylon SSR</h4>
								<div className="cmd">app/page.tsx</div>
								<p>Server-rendered React from the same server as your data. Streaming SSR, per-route code splitting, <code>&lt;Link&gt;</code> and <code>&lt;Image&gt;</code> built in. No separate Next.js process. <code>@pylonsync/next</code> is there if you want it.</p>
								<div className="footer-line"><span>react 19</span><span>streaming</span></div>
							</div>
							<div className="lane">
								<div className="top"><span><span className="num">WEB</span> · SPA</span><span>any bundler</span></div>
								<h4>React</h4>
								<div className="cmd">db.useQuery(&quot;Order&quot;)</div>
								<p>Subscription hooks that re-run on every relevant write. IndexedDB mirror answers reads in 0.4ms while the diff is in flight. Optimistic mutations with auto-rollback.</p>
								<div className="footer-line"><span>vite · rsbuild</span><span>optimistic</span></div>
							</div>
							<div className="lane">
								<div className="top"><span><span className="num">MOBILE</span> · CROSS</span><span>expo</span></div>
								<h4>Expo / RN</h4>
								<div className="cmd">@pylonsync/react-native</div>
								<p>The same hooks API, mobile-tuned. Hermes-friendly, background-safe sockets, on-device SQLite mirror so reads survive a bad cell tower.</p>
								<div className="footer-line"><span>ios + android</span><span>offline-first</span></div>
							</div>
							<div className="lane" style={{ borderRight: "none" }}>
								<div className="top"><span><span className="num">NATIVE</span> · IOS</span><span>swiftpm</span></div>
								<h4>Swift</h4>
								<div className="cmd">pylon codegen --target swift</div>
								<p>First-class Swift SDK with TypeScript-sync parity and a Loro CRDT bridge. Codegen turns your schema into typed entities and structured-concurrency clients.</p>
								<div className="footer-line"><span>actors · async</span><span>loro</span></div>
							</div>
						</div>
					</div>
				</section>

				{/* COMPARE */}
				<section className="block" id="compare">
					<div className="shell">
						<div className="eyebrow">How it compares</div>
						<h2 className="h2">Convex velocity. Rails ownership.</h2>
						<p className="section-lede">Pick a managed backend and you inherit its boundaries — and still bolt on a separate frontend framework. Pick raw infrastructure and you rebuild everything. Pylon is one model for the frontend and the backend, across both.</p>

						<div className="compare-wrap">
							<div className="compare">
								<table>
									<thead>
										<tr>
											<th>Capability</th>
											<th className="us">Pylon</th>
											<th>Convex</th>
											<th>Supabase</th>
											<th>Firebase</th>
										</tr>
									</thead>
									<tbody>
										<tr>
											<td className="label">Declarative schema</td>
											<td className="us"><span className="ind"><span className="dot-yes" />First-class</span></td>
											<td><span className="ind"><span className="dot-yes" />Yes</span></td>
											<td><span className="ind"><span className="dot-yes" />Yes</span></td>
											<td><span className="ind"><span className="dot-part" />Partial</span></td>
										</tr>
										<tr>
											<td className="label">Live queries with diffs</td>
											<td className="us"><span className="ind"><span className="dot-yes" />First-class</span></td>
											<td><span className="ind"><span className="dot-yes" />Yes</span></td>
											<td><span className="ind"><span className="dot-yes" />Yes</span></td>
											<td><span className="ind"><span className="dot-yes" />Yes</span></td>
										</tr>
										<tr>
											<td className="label">Server-rendered frontend</td>
											<td className="us"><span className="ind"><span className="dot-yes" />Built in</span></td>
											<td><span className="ind"><span className="dot-no" />—</span></td>
											<td><span className="ind"><span className="dot-no" />—</span></td>
											<td><span className="ind"><span className="dot-no" />—</span></td>
										</tr>
										<tr>
											<td className="label">TypeScript server functions</td>
											<td className="us"><span className="ind"><span className="dot-yes" />Native</span></td>
											<td><span className="ind"><span className="dot-yes" />Native</span></td>
											<td><span className="ind"><span className="dot-part" />Edge (Deno)</span></td>
											<td><span className="ind"><span className="dot-part" />Cloud Functions</span></td>
										</tr>
										<tr>
											<td className="label">Faceted search, no sidecar</td>
											<td className="us"><span className="ind"><span className="dot-yes" />BM25 in DB</span></td>
											<td><span className="ind"><span className="dot-no" />—</span></td>
											<td><span className="ind"><span className="dot-part" />pg_trgm</span></td>
											<td><span className="ind"><span className="dot-no" />—</span></td>
										</tr>
										<tr>
											<td className="label">Tick-based authoritative loop</td>
											<td className="us"><span className="ind"><span className="dot-yes" />20/30/60 tps</span></td>
											<td><span className="ind"><span className="dot-no" />—</span></td>
											<td><span className="ind"><span className="dot-no" />—</span></td>
											<td><span className="ind"><span className="dot-no" />—</span></td>
										</tr>
										<tr>
											<td className="label">Self-host as one binary</td>
											<td className="us"><span className="ind"><span className="dot-yes" />scp-able</span></td>
											<td><span className="ind"><span className="dot-part" />docker-compose</span></td>
											<td><span className="ind"><span className="dot-part" />multi-service</span></td>
											<td><span className="ind"><span className="dot-no" />Vendor only</span></td>
										</tr>
										<tr>
											<td className="label">Open source</td>
											<td className="us"><span className="ind"><span className="dot-yes" />MIT / Apache</span></td>
											<td><span className="ind"><span className="dot-yes" />Yes</span></td>
											<td><span className="ind"><span className="dot-yes" />Yes</span></td>
											<td><span className="ind"><span className="dot-no" />—</span></td>
										</tr>
									</tbody>
								</table>
							</div>
						</div>
					</div>
				</section>

				{/* DEPLOY LANES */}
				<section className="block" id="deploy" style={{ background: "var(--bg-alt)" }}>
					<div className="shell">
						<div className="eyebrow">Deploy</div>
						<h2 className="h2">One app, four ways to run it.</h2>
						<p className="section-lede">Pylon is not a hosting bet. The same frontend-and-backend app runs locally, on Pylon Cloud, on a VPS, or inside your AWS account — without rewriting handlers.</p>

						<div className="lanes">
							<div className="lane">
								<div className="top"><span><span className="num">01</span> · LOCAL</span><span>SQLite</span></div>
								<h4>pylon dev</h4>
								<div className="cmd">$ pylon dev</div>
								<p>SQLite backend, hot reload, type-safe client regen on every save. Zero config while you build.</p>
								<div className="footer-line"><span>hot reload</span><span>0 deps</span></div>
							</div>
							<div className="lane">
								<div className="top"><span><span className="num">02</span> · CLOUD</span><span>managed</span></div>
								<h4>Pylon Cloud</h4>
								<div className="cmd">$ pylon deploy</div>
								<p>Hosted infra when you want the framework, not another ops project. Same app, same APIs.</p>
								<div className="footer-line"><span>global</span><span>SOC2 in flight</span></div>
							</div>
							<div className="lane">
								<div className="top"><span><span className="num">03</span> · SELF-HOST</span><span>portable</span></div>
								<h4>Your infra</h4>
								<div className="cmd">$ docker run pylon</div>
								<p>Run the same binary on a VPS, container platform, or private network when control matters.</p>
								<div className="footer-line"><span>docker</span><span>systemd</span></div>
							</div>
							<div className="lane" style={{ borderRight: "none" }}>
								<div className="top"><span><span className="num">04</span> · AWS</span><span>terraform</span></div>
								<h4>ECS + Aurora</h4>
								<div className="cmd">$ terraform apply</div>
								<p>Move into your AWS account with Postgres, load balancing, secrets, and your VPC. Inherit none of the framework.</p>
								<div className="footer-line"><span>aws</span><span>sst · k8s</span></div>
							</div>
						</div>
					</div>
				</section>

				{/* QUICKSTART */}
				<section className="block">
					<div className="shell">
						<div className="eyebrow">Quickstart</div>
						<h2 className="h2">Four commands to a running app.</h2>
						<p className="section-lede">Frontend and backend live in minutes. Move to Pylon Cloud or your own infrastructure later — without changing a line of programming model.</p>

						<div className="qs-wrap">
							<div className="qs-term">
								<div className="head">
									<div className="dots"><i /><i /><i /></div>
									<span>my-app — pylon dev</span>
								</div>
								<div className="body">
									<div><span className="pr">❯</span> npm create @pylonsync/pylon@latest my-app</div>
									<div><span className="dim">  Creating my-app in ./my-app</span></div>
									<div><span className="ok">  ✓ Scaffolded api/ + web/ + shared schema</span></div>
									<div><span className="ok">  ✓ Installed @pylonsync/cli, sdk, react</span></div>
									<div><span className="pr">❯</span> cd my-app &amp;&amp; pylon dev</div>
									<div><span className="info">  → api  </span><span className="dim">http://localhost:4321</span></div>
									<div><span className="info">  → web  </span><span className="dim">http://localhost:3000</span></div>
									<div><span className="info">  → studio </span><span className="dim">http://localhost:4321/studio</span></div>
									<div><span className="ok">  ✓ Schema synced · 0 conflicts</span></div>
									<div><span className="ok">  ◉ Live · 0 clients · listening</span></div>
									<div><span className="pr">❯</span><span className="blink" /></div>
								</div>
							</div>

							<div className="qs-steps">
								<div className="qs-step"><div className="n">01</div><div><h5>Scaffold</h5><p>One npm command. Generates a Pylon backend + Next.js frontend in a single workspace — no global binary, no Rust toolchain, no Docker.</p></div></div>
								<div className="qs-step"><div className="n">02</div><div><h5>Install</h5><p>Pulls <code>@pylonsync/cli</code> (platform binary) plus the SDK and React bindings. Nothing global.</p></div></div>
								<div className="qs-step"><div className="n">03</div><div><h5>Run dev</h5><p>Spins up the API and web together. Watches your schema, regenerates the typed client on save.</p></div></div>
								<div className="qs-step"><div className="n">04</div><div><h5>Connect from React</h5><p>One <code>init</code> call, then <code>useQuery</code> subscribes and re-streams on every change.</p></div></div>
							</div>
						</div>
					</div>
				</section>

				{/* EXAMPLES */}
				<section className="block" id="examples">
					<div className="shell">
						<div className="eyebrow">Built with Pylon</div>
						<h2 className="h2">Real apps you can clone and break.</h2>
						<p className="section-lede">Every primitive shows up in a working example, MIT-licensed in the monorepo. Fork one, change the schema, and you have a head start.</p>

						<div className="prims" style={{ marginTop: 56 }}>
							<a className="prim" href="https://github.com/pylonsync/pylon/tree/main/examples/chat" style={{ textDecoration: "none" }}>
								<div className="top"><div className="icon">💬</div><div className="tag">app</div></div>
								<h4>Chat</h4>
								<p>Real-time messaging with rooms, presence, and typing indicators. Loro-backed conflict-free message ordering.</p>
							</a>
							<a className="prim" href="https://github.com/pylonsync/pylon/tree/main/examples/linear" style={{ textDecoration: "none" }}>
								<div className="top"><div className="icon">▶</div><div className="tag">app</div></div>
								<h4>Linear-style PM</h4>
								<p>Issues, projects, cycles, comments. Live cursors and optimistic mutations on every edit.</p>
							</a>
							<a className="prim" href="https://github.com/pylonsync/pylon/tree/main/examples/crm" style={{ textDecoration: "none" }}>
								<div className="top"><div className="icon">◆</div><div className="tag">app</div></div>
								<h4>CRM</h4>
								<p>Multi-tenant B2B contacts and pipelines. Demonstrates RBAC, audit logging, and the policy compiler.</p>
							</a>
							<a className="prim" href="https://github.com/pylonsync/pylon/tree/main/examples/world3d" style={{ textDecoration: "none" }}>
								<div className="top"><div className="icon">◉</div><div className="tag game">game</div></div>
								<h4>World 3D</h4>
								<p>Tick-based authoritative shard with area-of-interest replication. 60 tps in Rust, Three.js client.</p>
							</a>
							<a className="prim" href="https://github.com/pylonsync/pylon/tree/main/examples/swift-todo" style={{ textDecoration: "none" }}>
								<div className="top"><div className="icon">↥</div><div className="tag">native</div></div>
								<h4>Swift Todo</h4>
								<p>SwiftUI app over the native Pylon SDK. Showcases codegen, structured concurrency, and the Loro bridge.</p>
							</a>
							<a className="prim" href="https://github.com/pylonsync/pylon/tree/main/examples/store" style={{ textDecoration: "none" }}>
								<div className="top"><div className="icon">⌕</div><div className="tag">app</div></div>
								<h4>Store</h4>
								<p>E-commerce with faceted search, files, and a Stripe-style checkout workflow. BM25 over the catalog.</p>
							</a>
						</div>

						<div style={{ marginTop: 32, display: "flex", justifyContent: "center" }}>
							<Link className="btn line" href="/showcase">View all 16 examples →</Link>
						</div>
					</div>
				</section>

				{/* AI AGENT SKILL */}
				<section className="block" id="skill" style={{ background: "var(--bg-alt)" }}>
					<div className="shell">
						<div className="eyebrow">Claude Code skill</div>
						<h2 className="h2">Your coding agent already knows Pylon.</h2>
						<p className="section-lede">One markdown file teaches Claude Code the schema model, the policy DSL, the server-function runtime, and the React client. Drop it into <code>~/.claude/skills/pylon/</code> and Claude generates code that compiles instead of code that looks like it should.</p>

						<div className="qs-wrap" style={{ marginTop: 64 }}>
							<div className="qs-term">
								<div className="head">
									<div className="dots"><i /><i /><i /></div>
									<span>install pylon skill</span>
								</div>
								<div className="body">
									<div><span className="pr">❯</span> mkdir -p ~/.claude/skills/pylon</div>
									<div><span className="pr">❯</span> curl -fsSL https://pylonsync.com/pylon-skill.md \</div>
									<div>{"      "}&gt; ~/.claude/skills/pylon/SKILL.md</div>
									<div><span className="ok">  ✓ Wrote 523 lines</span></div>
									<div><span className="dim">    Schema · policies · functions · React · deploy</span></div>
									<div>&nbsp;</div>
									<div><span className="dim">  # Restart Claude Code; the skill auto-loads</span></div>
									<div><span className="dim">  # whenever you work on a Pylon project.</span></div>
									<div><span className="pr">❯</span><span className="blink" /></div>
								</div>
							</div>

							<div className="qs-steps">
								<div className="qs-step"><div className="n">01</div><div><h5>One file, the whole framework</h5><p>523 lines of conventions — schema shape, policy expressions, <code>ctx.*</code> helpers, manifest behaviors, deploy paths. The same handbook the maintainers write to.</p></div></div>
								<div className="qs-step"><div className="n">02</div><div><h5>User-wide or project-scoped</h5><p>Save to <code>~/.claude/skills/pylon/SKILL.md</code> for every project, or <code>.claude/skills/pylon/SKILL.md</code> committed alongside one app.</p></div></div>
								<div className="qs-step"><div className="n">03</div><div><h5>Stays current with the framework</h5><p>The skill lives at <code>pylonsync.com/pylon-skill.md</code> and ships with each Pylon release. Re-curl when a new version drops.</p></div></div>
								<div className="qs-step"><div className="n">→</div><div><h5>Read it first</h5><p>Full install instructions and the whole skill rendered inline at <a href="/skill" style={{ color: "var(--accent)" }}>pylonsync.com/skill</a>.</p></div></div>
							</div>
						</div>
					</div>
				</section>

				{/* BIG CTA */}
				<section className="cta-block">
					<div className="bg-grid" />
					<div className="accent-glow" />
					<div className="shell">
						<div className="inner">
							<h2>Stop gluing services together.</h2>
							<p>Open source, MIT/Apache. Free tier on Pylon Cloud — pay when you outgrow it, or take the binary and run it yourself.</p>
							<div className="ctas">
								<Link className="btn accent" href="https://cloud.pylonsync.com">Start free →</Link>
								<a className="btn ghost" style={{ color: "rgba(255,255,255,.7)" }} href="https://docs.pylonsync.com">Read the docs</a>
							</div>
							<div className="cta-secondary">
								<span>Or run locally:</span>
								<CopyCommand command="npm create @pylonsync/pylon@latest" />
							</div>
						</div>
					</div>
				</section>

				{/* FOOTER */}
				<footer>
					<div className="shell foot-grid">
						<div className="brand-col">
							<a className="brand" href="#">
								<PylonMark size={20} style={{ color: "var(--ink)" }} />
								Pylon
							</a>
							<p>The full-stack realtime framework for TypeScript apps. Server-rendered React, schema, server functions, live queries, auth, jobs, files, and search — frontend and backend in one server.</p>
						</div>
						<div>
							<h6>Product</h6>
							<ul>
								<li><a href="#features">Overview</a></li>
								<li><a href="https://docs.pylonsync.com/concepts/live-queries">Live queries</a></li>
								<li><a href="https://docs.pylonsync.com/plugins/overview">Plugins</a></li>
								<li><a href="#compare">Compare</a></li>
								<li><a href="https://cloud.pylonsync.com">Pylon Cloud</a></li>
							</ul>
						</div>
						<div>
							<h6>Developers</h6>
							<ul>
								<li><a href="https://docs.pylonsync.com">Documentation</a></li>
								<li><a href="https://docs.pylonsync.com/quickstart">Quickstart</a></li>
								<li><a href="https://docs.pylonsync.com/operations/vercel">Deploy to Vercel</a></li>
								<li><a href="https://github.com/pylonsync/pylon/tree/main/examples">Examples</a></li>
								<li><a href="/skill">Claude Code skill</a></li>
								<li><a href="https://github.com/pylonsync/pylon/releases">Changelog</a></li>
							</ul>
						</div>
						<div>
							<h6>Resources</h6>
							<ul>
								<li><a href="https://github.com/pylonsync/pylon">GitHub</a></li>
								<li><a href="https://github.com/pylonsync/pylon/discussions">Discussions</a></li>
								<li><a href="https://github.com/pylonsync/pylon/blob/main/SECURITY.md">Security</a></li>
							</ul>
						</div>
						<div className="foot-meta" style={{ gridColumn: "1 / -1" }}>
							<span>© 2026 Pylon Labs, Inc · MIT / Apache-2.0 dual-licensed</span>
							<span className="status">All systems operational · v{version}</span>
						</div>
					</div>
				</footer>
			</div>
		</>
	);
}
