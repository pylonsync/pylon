import type * as React from "react";
import { cn } from "../lib/utils";

// Pylon brand marks. Inline SVG so the mark inherits `currentColor` —
// renders correctly in either theme without an extra request and
// without `dark:invert` luminance flips. Mirrors the marketing site's
// `<PylonMark>` component (apps/web/components/pylon-logo.tsx in the
// pylon repo) so cloud + landing share the same mark shape.

export function PylonMark({
	className,
	size = 18,
	...rest
}: React.SVGProps<SVGSVGElement> & { size?: number }) {
	return (
		<svg
			viewBox="0 0 48 64"
			width={size}
			height={(size * 64) / 48}
			fill="currentColor"
			xmlns="http://www.w3.org/2000/svg"
			className={cn("shrink-0 select-none", className)}
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

// Lockup is the wordmark — used on the landing page + auth pages where
// we want the full brand. The header inside the dashboard uses the
// icon mark + "Cloud" suffix instead.
export function PylonWordmark({
	className,
	size = 22,
}: {
	className?: string;
	size?: number;
}) {
	return (
		<span
			className={cn(
				"inline-flex items-center gap-2.5 text-[var(--color-ink)]",
				className,
			)}
			style={{
				fontWeight: 500,
				fontSize: size * 0.8,
				letterSpacing: "-0.015em",
			}}
		>
			<PylonMark size={size} />
			<span className="leading-none">Pylon</span>
		</span>
	);
}

// Compact lockup used in the dashboard header: icon + "Pylon Cloud".
// "Pylon" carries the brand weight; "Cloud" dims to mark the product
// surface without competing visually.
export function PylonLockup({ className }: { className?: string }) {
	return (
		<span
			className={cn(
				"inline-flex items-center gap-2 text-[var(--color-ink)]",
				className,
			)}
		>
			<PylonMark size={18} />
			<span className="text-[14px] font-semibold tracking-tight leading-none">
				Pylon{" "}
				<span className="text-[var(--color-ink-3)] font-normal">Cloud</span>
			</span>
		</span>
	);
}
