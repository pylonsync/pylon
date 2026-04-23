---
version: alpha
name: Stage Editor
description: Canvas-first site builder. Confident typography, generous whitespace, a single warm accent that does the work.
colors:
  primary: "#0B0B0F"
  on-primary: "#FFFFFF"
  secondary: "#5B5B64"
  tertiary: "#FF3D7F"
  on-tertiary: "#FFFFFF"
  tertiary-container: "#FFE1EC"
  on-tertiary-container: "#3D0E23"
  neutral: "#F5F4F7"
  neutral-strong: "#E9E7EE"
  surface: "#FFFFFF"
  surface-raised: "#FAFAFC"
  surface-sunken: "#F2F1F6"
  canvas: "#FFFFFF"
  outline: "#E6E4EB"
  outline-strong: "#C9C6D1"
  text: "#0B0B0F"
  text-muted: "#5B5B64"
  text-dim: "#8C8A96"
  success: "#00A86B"
  warning: "#F2994A"
  danger: "#E5484D"
  cursor-1: "#FF3D7F"
  cursor-2: "#7C5CFF"
  cursor-3: "#F2994A"
  cursor-4: "#00A86B"
  cursor-5: "#2D9CDB"
  cursor-6: "#E5484D"
typography:
  display:
    fontFamily: Inter
    fontSize: 3rem
    fontWeight: 700
    letterSpacing: -0.025em
    lineHeight: 1.05
  h1:
    fontFamily: Inter
    fontSize: 2rem
    fontWeight: 700
    letterSpacing: -0.02em
    lineHeight: 1.1
  h2:
    fontFamily: Inter
    fontSize: 1.375rem
    fontWeight: 600
    letterSpacing: -0.01em
    lineHeight: 1.2
  h3:
    fontFamily: Inter
    fontSize: 1rem
    fontWeight: 600
    letterSpacing: -0.005em
    lineHeight: 1.3
  body-lg:
    fontFamily: Inter
    fontSize: 1rem
    fontWeight: 400
    lineHeight: 1.55
  body-md:
    fontFamily: Inter
    fontSize: 0.8125rem
    fontWeight: 400
    lineHeight: 1.5
  body-sm:
    fontFamily: Inter
    fontSize: 0.75rem
    fontWeight: 400
    lineHeight: 1.4
  label:
    fontFamily: Inter
    fontSize: 0.75rem
    fontWeight: 500
    letterSpacing: 0.01em
  label-caps:
    fontFamily: Inter
    fontSize: 0.6875rem
    fontWeight: 600
    letterSpacing: 0.08em
  code:
    fontFamily: JetBrains Mono
    fontSize: 0.75rem
    fontWeight: 400
rounded:
  xs: 4px
  sm: 6px
  md: 8px
  lg: 12px
  xl: 16px
  "2xl": 20px
  pill: 999px
spacing:
  "0": 0px
  "1": 2px
  "2": 4px
  "3": 8px
  "4": 12px
  "5": 16px
  "6": 20px
  "7": 24px
  "8": 32px
  "9": 48px
  "10": 64px
elevation:
  "1": "0 1px 2px rgba(11, 11, 15, 0.06)"
  "2": "0 4px 16px -6px rgba(11, 11, 15, 0.12)"
  "3": "0 20px 48px -16px rgba(11, 11, 15, 0.22)"
motion:
  fast: 100ms
  base: 160ms
  slow: 260ms
  ease: cubic-bezier(0.2, 0.8, 0.2, 1)
components:
  button-primary:
    backgroundColor: "{colors.tertiary}"
    textColor: "{colors.on-tertiary}"
    rounded: "{rounded.md}"
    padding: 10px 14px
    typography: "{typography.label}"
  button-primary-hover:
    backgroundColor: "#E62F6E"
  button-ghost:
    backgroundColor: transparent
    textColor: "{colors.text-muted}"
    rounded: "{rounded.md}"
    padding: 8px 12px
  button-ghost-hover:
    backgroundColor: "{colors.neutral}"
    textColor: "{colors.text}"
  chip:
    backgroundColor: "{colors.neutral}"
    textColor: "{colors.text-muted}"
    rounded: "{rounded.pill}"
    padding: 2px 10px
    typography: "{typography.label}"
  card:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.lg}"
  input:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.text}"
    rounded: "{rounded.sm}"
    padding: 8px 10px
    typography: "{typography.body-md}"
---

## Overview

Stage is a site builder. The UI is a quiet tool that gets out of the way so the user's content is the loudest thing on screen. Canvas white, soft neutral shell, one warm accent (`tertiary`) that carries every moment of emphasis — hovered blocks, the primary action, live cursors, the streak of selected state.

Every surface earns its line. Outlines over fills, hairline strokes (1px `outline`), rounded-md (8px) as the default, rounded-lg (12px) for containers the user lives in. Motion is fast (100–160ms) with a single easing curve — the feel of something built for power users, not impressed visitors.

## Colors

**Neutrals** carry 90% of the UI. `surface` is the canvas; `surface-raised` is inspector fields and cards; `surface-sunken` is the page behind the canvas. Outlines in `outline` (hairline) and `outline-strong` (hover).

**Tertiary** (`#FF3D7F`) is the only saturated hex in the system. It marks the primary button, the selected block outline, the "Live" dot, the streak on an active nav item. If two things on screen both wear it, one of them is wrong.

**Cursor palette** (`cursor-1`..`cursor-6`) deterministically assigns a color to each collaborator by hashing their user id. They never touch interactive state — only presence.

## Typography

Inter at every size — variable weight, optical sizing. `display` is reserved for splash / empty states. `h1` is page titles in the dashboard, block title when selected. `h2` is section headers. `body-md` is the default — 13px feels tight at first but matches the density of Figma/Linear/Framer once you live in it.

`label-caps` carries section labels in the nav and inspector. Tracked out, caps, no color — an organizing cue rather than text to read.

Monospace is `JetBrains Mono`, reserved for slugs and code snippets.

## Layout

Three-pane editor: 240px left nav, flexible canvas center, 280px right inspector. The canvas has a 820px-max content column; breakpoint tabs shrink that max to 1024 / 720 / 390 without zooming — content really reflows so the user sees mobile as users will.

Spacing scale is linear-ish: 2, 4, 8, 12, 16, 20, 24, 32, 48, 64. Tight enough to pack the inspector, loose enough that the canvas breathes.

## Elevation & Depth

Three levels — `1` for inline cards, `2` for floating popovers and site cards on the dashboard, `3` for modals. No drop shadows on nav items or buttons; hover state lives in background swap + outline, not elevation.

## Shapes

`rounded.md` (8px) is the default. Buttons, inputs, nav items, chips. `rounded.lg` (12px) on cards + the canvas itself. `rounded.xl` (16px) on modals. `rounded.pill` on status badges and the cursor name tag.

## Components

**button-primary** — tertiary background, 10×14 padding, label weight. One per screen.
**button-ghost** — transparent, muted text; hover into neutral fill. All secondary actions.
**chip** — pill, body-md type, status marker. Draft/Live, published timestamps.
**card** — surface over sunken page. No visible outline unless hovered.
**input** — surface-raised with hairline outline. Focus ring = tertiary.

## Do's and Don'ts

**Do** rely on hairline outlines over fills to separate regions.
**Do** use tertiary sparingly — one moment of attention at a time.
**Do** animate selection with the 160ms / ease curve.

**Don't** nest elevation — a card inside a card should be a sunken surface, not stacked shadows.
**Don't** introduce a second accent for "variety"; reach for a neutral tier instead.
**Don't** use typography weight to express hierarchy when color or size already does.
