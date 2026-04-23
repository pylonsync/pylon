/**
 * Pylon logo mark. Uses `currentColor` so it inherits from parent via
 * `color` CSS — renders white in the dark nav, black on light surfaces,
 * gradient-able with `background-clip: text` on the parent if needed.
 *
 * Placeholder vector — when you finalize the mark in Figma, export as
 * SVG and replace the <path> contents below. The viewBox + currentColor
 * plumbing is already wired up, so the swap is one path.
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
      {/* Right crystal facet (slight shade via opacity if using dual-tone) */}
      <path d="M24 2 L38 20 L24 32 Z" />
      {/* Crystal lower wedge (divides into left/right halves meeting at centerline) */}
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      {/* Left cradle arm */}
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      {/* Right cradle arm */}
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}

// Horizontal mark + wordmark for nav / inline use.
export function PylonLogo({ className }: { className?: string }) {
  return (
    <span
      className={className}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 10,
        fontWeight: 600,
        fontSize: 16,
        letterSpacing: "-0.015em",
      }}
    >
      <PylonMark size={22} />
      <span>Pylon</span>
    </span>
  );
}
