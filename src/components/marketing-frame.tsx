import type React from "react";

/**
 * The frame every marketing band aligns to.
 *
 * The two hairlines are `border-x` on each band's content column, not a
 * single full-height overlay behind the page. An overlay has to sit behind
 * the bands to avoid drawing over their cards, and every band paints its own
 * background — so the overlay is always covered. Putting the rules on the
 * content column means each band draws its own segment, and because every
 * band shares this measure the segments line up into one continuous pair.
 *
 * Any new band must use this, or the rules break at that section.
 */
export const FRAME_COL =
	"mx-auto w-full max-w-[1280px] border-x border-[var(--color-rule)]";

/**
 * A page band: full-bleed background, content held to the frame measure.
 *
 * `tone` sets the chapter. `paper` and `sunken` are the quiet pair for
 * adjacent sections; `ink` is the hard cut — a dark band that tells the
 * reader a new chapter started. Alternating paper/sunken alone is close to
 * invisible, which is how a long page turns into one undifferentiated scroll.
 */
export function Band({
	id,
	tone = "paper",
	label,
	className = "",
	children,
}: {
	id?: string;
	tone?: "paper" | "sunken" | "ink";
	/** Small mono marker in the band's top-left corner, e.g. "TEMPLATES". */
	label?: string;
	className?: string;
	children: React.ReactNode;
}) {
	const tones = {
		paper: "bg-[var(--color-paper)] text-[var(--color-ink)]",
		sunken: "bg-[var(--color-paper-1)] text-[var(--color-ink)]",
		ink: "bg-[var(--color-ink)] text-[var(--color-paper)]",
	};

	return (
		<section
			id={id}
			className={`border-t border-[var(--color-rule)] ${tones[tone]} ${className}`}
		>
			{label ? (
				// Corner marker, not an eyebrow: it names the band and sits in the
				// gutter above the content, never stacked on a heading.
				<div
					className={`${FRAME_COL} border-b border-[var(--color-rule)] px-5 py-3 sm:px-8`}
				>
					<span className="font-mono text-[10.5px] uppercase tracking-[0.14em] opacity-45">
						{label}
					</span>
				</div>
			) : null}
			<div className={`${FRAME_COL} px-5 py-16 sm:px-8 sm:py-20 lg:py-24`}>
				{children}
			</div>
		</section>
	);
}
