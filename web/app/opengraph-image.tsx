import { ImageResponse } from "@pylonsync/react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The OG card for pylonsync.com, rendered by the framework.
//
// This used to be a PNG committed next to this file, screenshotted by hand out
// of headless Chrome from a sibling .source.html. That pipeline has one failure
// mode and the repo already hit it: apps/control-plane's source HTML was edited
// to say usesmallware.com and the committed PNG — still reading pylonsync.com —
// was never re-rendered, so the product's card advertised the wrong domain.
// Nothing here can drift from itself.
//
// Satori is flexbox-only (same rule as next/og): every element with more than
// one child sets `display: flex` explicitly. There is no inline layout, so a
// line of code is a flex row and each colored token is its own child.

export const size = { width: 1200, height: 630 };

// Site tokens — apps/pylonsync-site/web/app/globals.css.
const PAPER = "#ffffff";
const PAPER_1 = "#fafafa";
const RULE = "#e4e4e7";
const INK = "#18181b";
const INK_2 = "#3f3f46";
const INK_3 = "#71717a";
const INK_4 = "#a1a1aa";
const COBALT = "#1d4ed8";
const LIVE = "#059669";

const SANS = "Geist";
const MONO = "Geist Mono";

// Satori needs static TTF buffers — it cannot parse woff2 and a variable font
// crashes it, so the app's usual self-hosted woff2 faces are no use here. These
// four static cuts sit next to this module and ship with `web/` in the image.
// Without them the framework falls back to its bundled Inter, which would put
// the card in a different typeface than the page it links to.
const font = (file: string) =>
	readFileSync(fileURLToPath(new URL(`./_og-fonts/${file}`, import.meta.url)));

/** One syntax-colored token. Spaces are significant, so nothing collapses. */
function Tok({ t, c }: { t: string; c: string }) {
	return <span style={{ color: c, whiteSpace: "pre" }}>{t}</span>;
}

/** A line of the sample. Colors mirror highlightLine() in code-panel.tsx:
 *  comment → ink-4, string → status-live, keyword → cobalt, identifier →
 *  ink-2, punctuation → ink-3. */
function Line({ children }: { children: React.ReactNode }) {
	return <div style={{ display: "flex", height: 31 }}>{children}</div>;
}

export default function OpengraphImage() {
	return new ImageResponse(
		<div
			style={{
				width: "100%",
				height: "100%",
				display: "flex",
				background: PAPER,
				color: INK,
				fontFamily: SANS,
				// The card needs an edge — pure white bleeds into Slack's and X's
				// chrome — but it's the same 1px rule the site uses everywhere.
				border: `1px solid ${RULE}`,
			}}
		>
			{/* Left column */}
			<div
				style={{
					width: 640,
					display: "flex",
					flexDirection: "column",
					padding: "62px 0 62px 64px",
				}}
			>
				<div style={{ display: "flex", alignItems: "center" }}>
					{/* Ink, never tinted. DESIGN.md: the mark keeps its own value and
					    the chrome around it carries the color. */}
					<svg width="38" height="51" viewBox="0 0 48 64" fill={INK}>
						<path d="M24 2 L10 20 L24 32 Z" />
						<path d="M24 2 L38 20 L24 32 Z" />
						<path d="M24 32 L18 48 L24 62 L30 48 Z" />
						<path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
						<path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
					</svg>
					<span
						style={{
							marginLeft: 16,
							fontSize: 37,
							fontWeight: 700,
							letterSpacing: -1.1,
						}}
					>
						Pylon
					</span>
				</div>

				{/* Breaks are explicit. No single measure reproduces the page's
				    15 / 13 / 9: anything wide enough to hold "Full-stack apps" on
				    one line also pulls "can" up onto the accent line and orphans
				    "ship." The phrase "coding agents" has to stay whole. */}
				<div
					style={{
						marginTop: 50,
						display: "flex",
						flexDirection: "column",
						fontSize: 62,
						fontWeight: 600,
						letterSpacing: -2.2,
						lineHeight: 1.02,
					}}
				>
					<span>Full-stack apps that</span>
					<span style={{ color: COBALT }}>coding agents</span>
					<span>can ship.</span>
				</div>

				<div
					style={{
						marginTop: 26,
						width: 430,
						fontSize: 23,
						lineHeight: 1.45,
						color: INK_2,
					}}
				>
					Pylon is a full-stack framework built for agents to ship
					high-performance and secure apps quickly.
				</div>

				{/* Closes on the command that starts the thing. Every OG client
				    already prints the domain under the card, so the old
				    "— pylonsync.com" line was spending the close on a duplicate. */}
				<div
					style={{
						marginTop: "auto",
						display: "flex",
						alignItems: "center",
						alignSelf: "flex-start",
						height: 52,
						padding: "0 22px",
						border: `1px solid ${RULE}`,
						borderRadius: 999,
						fontFamily: MONO,
						fontSize: 20,
						color: INK,
					}}
				>
					<span style={{ color: COBALT }}>$</span>
					<span style={{ marginLeft: 12 }}>npm create @pylonsync/pylon</span>
				</div>
			</div>

			{/* The product, cropped by the card's right edge — the same "window onto
			    something real" the artifact cards on the page use, and it fills the
			    40% the old card left empty. Top edge sits on the wordmark's top,
			    bottom on the command pill's bottom, so the crop reads as intended
			    rather than as a clipped element. */}
			<div
				style={{
					width: 560,
					margin: "62px 0 62px 24px",
					display: "flex",
					flexDirection: "column",
					background: PAPER_1,
					border: `1px solid ${RULE}`,
					borderRight: "none",
					borderTopLeftRadius: 16,
					borderBottomLeftRadius: 16,
				}}
			>
				<div
					style={{
						display: "flex",
						padding: "13px 22px",
						borderBottom: `1px solid ${RULE}`,
						fontFamily: MONO,
						fontSize: 15,
						color: INK_3,
					}}
				>
					app.ts
				</div>
				<div
					style={{
						display: "flex",
						flexDirection: "column",
						padding: "20px 22px",
						fontFamily: MONO,
						fontSize: 18,
						color: INK_2,
					}}
				>
					<Line>
						<Tok t="// one entity → table, API, typed client" c={INK_4} />
					</Line>
					<Line>
						<Tok t="const " c={COBALT} />
						<Tok t="Order" c={INK_2} />
						<Tok t=" = " c={INK_3} />
						<Tok t="entity" c={COBALT} />
						<Tok t="(" c={INK_3} />
						<Tok t={'"Order"'} c={LIVE} />
						<Tok t=", {" c={INK_3} />
					</Line>
					<Line>
						<Tok t="  customer" c={INK_2} />
						<Tok t=": " c={INK_3} />
						<Tok t="field" c={COBALT} />
						<Tok t="." c={INK_3} />
						<Tok t="string" c={INK_2} />
						<Tok t="()," c={INK_3} />
					</Line>
					<Line>
						<Tok t="  total" c={INK_2} />
						<Tok t=": " c={INK_3} />
						<Tok t="field" c={COBALT} />
						<Tok t="." c={INK_3} />
						<Tok t="float" c={INK_2} />
						<Tok t="()," c={INK_3} />
					</Line>
					<Line>
						<Tok t="});" c={INK_3} />
					</Line>
					<Line>
						<Tok t=" " c={INK_3} />
					</Line>
					<Line>
						<Tok t="// access rules next to the schema" c={INK_4} />
					</Line>
					<Line>
						<Tok t="policy" c={COBALT} />
						<Tok t="({ " c={INK_3} />
						<Tok t="entity" c={COBALT} />
						<Tok t=": " c={INK_3} />
						<Tok t={'"Order"'} c={LIVE} />
						<Tok t="," c={INK_3} />
					</Line>
					<Line>
						<Tok t="  allowRead" c={INK_2} />
						<Tok t=": " c={INK_3} />
						<Tok t={'"auth.userId != null"'} c={LIVE} />
						<Tok t="," c={INK_3} />
					</Line>
					<Line>
						<Tok t="});" c={INK_3} />
					</Line>
					<Line>
						<Tok t=" " c={INK_3} />
					</Line>
					<Line>
						<Tok t="const " c={COBALT} />
						<Tok t="{ " c={INK_3} />
						<Tok t="data" c={INK_2} />
						<Tok t=" } = " c={INK_3} />
						<Tok t="db" c={INK_2} />
						<Tok t="." c={INK_3} />
						<Tok t="useQuery" c={INK_2} />
						<Tok t="(" c={INK_3} />
						<Tok t={'"Order"'} c={LIVE} />
						<Tok t=");" c={INK_3} />
					</Line>
				</div>
			</div>
		</div>,
		{
			...size,
			// No `headers` override on purpose. The OG route already defaults to
			// `public, max-age=3600, s-maxage=86400` and the on-disk ISR layer keys
			// by URL, so the render happens once per deploy, not once per crawler.
			// (`pylon dev` forces `private, no-store` on every response — that's the
			// dev live-reload default, not this route's production behavior.)
			fonts: [
				{ name: SANS, data: font("Geist-400.ttf"), weight: 400, style: "normal" },
				{ name: SANS, data: font("Geist-600.ttf"), weight: 600, style: "normal" },
				{ name: SANS, data: font("Geist-700.ttf"), weight: 700, style: "normal" },
				{ name: MONO, data: font("GeistMono-400.ttf"), weight: 400, style: "normal" },
			],
		},
	);
}
