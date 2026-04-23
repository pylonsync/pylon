/**
 * Pylon logo mark. Inline SVG so it inherits currentColor — renders
 * white in the dark nav, black on light surfaces, no extra request.
 *
 * The `/public/brand/` folder contains generated alternate assets
 * (pylon-icon.svg, pylon-logo.svg, PNG @1x/@2x/@3x) if we ever want
 * to swap to those; they're linked for favicon + OG use.
 */
import * as React from "react";

export function PylonMark({
  size = 24,
  className,
  ...rest
}: React.SVGProps<SVGSVGElement> & { size?: number }) {
  return (
    <svg
      viewBox="0 0 48 64"
      width={size}
      height={(size * 64) / 48}
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-label="Pylon"
      {...rest}
    >
      {/* Left crystal facet */}
      <path d="M24 2 L10 20 L24 32 Z" />
      {/* Right crystal facet */}
      <path d="M24 2 L38 20 L24 32 Z" />
      {/* Crystal lower rhombus */}
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      {/* Left cradle arm */}
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      {/* Right cradle arm */}
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}

export function PylonLogo({ className }: { className?: string }) {
  return (
    <span
      className={className}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 10,
        fontWeight: 500,
        fontSize: 16,
        letterSpacing: "-0.015em",
      }}
    >
      <PylonMark size={22} />
      <span>Pylon</span>
    </span>
  );
}
