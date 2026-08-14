import { ImageResponse } from "@pylonsync/react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export const size = { width: 1200, height: 630 };

const PAPER = "#ffffff";
const PAPER_1 = "#fafafa";
const RULE = "#e4e4e7";
const INK = "#18181b";
const INK_2 = "#3f3f46";
const INK_3 = "#71717a";
const BRAND = "#6d4aff";
const BRAND_SOFT = "#eeeaff";

const SANS = "Geist";
const MONO = "Geist Mono";

const font = (file: string) =>
	readFileSync(fileURLToPath(new URL(`./_og-fonts/${file}`, import.meta.url)));

function PylonMark({ size: markSize, color = INK }: { size: number; color?: string }) {
	return (
		<svg width={markSize} height={(markSize * 4) / 3} viewBox="0 0 48 64" fill={color}>
			<path d="M24 2 L10 20 L24 32 Z" />
			<path d="M24 2 L38 20 L24 32 Z" />
			<path d="M24 32 L18 48 L24 62 L30 48 Z" />
			<path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
			<path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
		</svg>
	);
}

function RuntimeBoard() {
	const nodes = [
		{ x: 138, y: 168 },
		{ x: 228, y: 116 },
		{ x: 318, y: 168 },
		{ x: 138, y: 272 },
		{ x: 228, y: 324 },
		{ x: 318, y: 272 },
	];

	return (
		<div
			style={{
				width: "100%",
				height: "100%",
				display: "flex",
				alignItems: "center",
				justifyContent: "center",
				background: PAPER_1,
				position: "relative",
			}}
		>
			<svg width="535" height="470" viewBox="0 0 535 470">
				<defs>
					<filter id="board-shadow" x="-20%" y="-20%" width="140%" height="160%">
						<feDropShadow dx="0" dy="14" stdDeviation="13" floodColor="#6d4aff" floodOpacity="0.12" />
					</filter>
				</defs>

				<path d="M228 76 L432 193 L228 310 L24 193 Z" fill="#e8e2ff" stroke="#d8ceff" />
				<path d="M228 58 L432 175 L228 292 L24 175 Z" fill={PAPER} stroke="#d4d4d8" filter="url(#board-shadow)" />

				<path d="M228 83 L388 175 L228 267 L68 175 Z" fill="none" stroke="#e4e4e7" strokeDasharray="4 7" />
				<path d="M228 109 L342 175 L228 241 L114 175 Z" fill="none" stroke="#ededf0" strokeDasharray="4 7" />
				<path d="M228 135 L297 175 L228 215 L159 175 Z" fill="none" stroke="#ededf0" strokeDasharray="4 7" />

				{nodes.map((node) => (
					<path
						key={`${node.x}-${node.y}`}
						d={`M228 175 L${node.x} ${node.y}`}
						stroke="#d4d4d8"
						strokeDasharray="3 5"
					/>
				))}
				<path d="M228 175 L318 168" stroke={BRAND} strokeWidth="2" />

				{nodes.map((node, index) => (
					<g key={`node-${node.x}-${node.y}`}>
						<path
							d={`M${node.x} ${node.y - 11} L${node.x + 19} ${node.y} L${node.x} ${node.y + 11} L${node.x - 19} ${node.y} Z`}
							fill={index === 2 ? BRAND : PAPER}
							stroke={index === 2 ? BRAND : "#a1a1aa"}
							strokeWidth="1.4"
						/>
						{index !== 2 && <circle cx={node.x} cy={node.y} r="2.5" fill="#a1a1aa" />}
					</g>
				))}

				<ellipse cx="228" cy="175" rx="66" ry="38" fill={BRAND_SOFT} />
				<path d="M228 145 L274 171 L228 197 L182 171 Z" fill={PAPER} stroke={BRAND} strokeWidth="2" />
				<g transform="translate(214 151) scale(0.58)" fill={BRAND}>
					<path d="M24 2 L10 20 L24 32 Z" />
					<path d="M24 2 L38 20 L24 32 Z" />
					<path d="M24 32 L18 48 L24 62 L30 48 Z" />
					<path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
					<path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
				</g>

				<path d="M138 168 L58 122 L8 122" fill="none" stroke="#d4d4d8" strokeDasharray="3 5" />
				<path d="M138 272 L58 318 L8 318" fill="none" stroke="#d4d4d8" strokeDasharray="3 5" />
				<path d="M318 168 L408 116 L520 116" fill="none" stroke={BRAND} strokeWidth="1.6" />
				<path d="M318 272 L408 324 L520 324" fill="none" stroke="#d4d4d8" strokeDasharray="3 5" />

			</svg>
			<span style={{ position: "absolute", top: 78, left: 24, fontFamily: MONO, fontSize: 12, color: INK_3, letterSpacing: 1.6 }}>TYPED SCHEMA</span>
			<span style={{ position: "absolute", top: 313, left: 24, fontFamily: MONO, fontSize: 12, color: INK_3, letterSpacing: 1.6 }}>DATABASE + FILES</span>
			<span style={{ position: "absolute", top: 72, right: 28, fontFamily: MONO, fontSize: 12, color: BRAND, letterSpacing: 1.6 }}>LIVE QUERIES</span>
			<span style={{ position: "absolute", top: 317, right: 24, fontFamily: MONO, fontSize: 12, color: INK_3, letterSpacing: 1.6 }}>SERVER FUNCTIONS</span>
			<span style={{ position: "absolute", bottom: 36, left: 204, fontFamily: MONO, fontSize: 11, color: INK_3, letterSpacing: 1.5 }}>ONE SCHEMA. ONE SERVER.</span>
		</div>
	);
}

export default function OpengraphImage() {
	return new ImageResponse(
		<div
			style={{
				width: "100%",
				height: "100%",
				display: "flex",
				flexDirection: "column",
				background: PAPER,
				color: INK,
				fontFamily: SANS,
				border: `1px solid ${RULE}`,
			}}
		>
			<div
				style={{
					height: 80,
					display: "flex",
					alignItems: "center",
					justifyContent: "space-between",
					padding: "0 48px",
					borderBottom: `1px solid ${RULE}`,
				}}
			>
				<div style={{ display: "flex", alignItems: "center" }}>
					<PylonMark size={25} />
					<span style={{ marginLeft: 12, fontSize: 26, fontWeight: 700, letterSpacing: -0.7 }}>Pylon</span>
				</div>
				<span style={{ fontFamily: MONO, fontSize: 14, color: INK_3, letterSpacing: 1.5 }}>PYLONSYNC.COM</span>
			</div>

			<div style={{ display: "flex", height: 550 }}>
				<div
					style={{
						width: 600,
						display: "flex",
						flexDirection: "column",
						padding: "56px 52px 48px 48px",
					}}
				>
					<div
						style={{
							display: "flex",
							flexDirection: "column",
							fontSize: 65,
							fontWeight: 600,
							letterSpacing: -2.8,
							lineHeight: 0.98,
						}}
					>
						<span>Give your agent</span>
						<span style={{ color: BRAND }}>app building</span>
						<span style={{ color: BRAND }}>superpowers</span>
					</div>

					<div style={{ marginTop: 27, width: 495, fontSize: 22, lineHeight: 1.42, color: INK_2 }}>
						A full-stack framework for agents to ship secure, high-performance apps.
					</div>

					<div
						style={{
							marginTop: "auto",
							display: "flex",
							alignItems: "center",
							alignSelf: "flex-start",
							height: 52,
							padding: "0 20px",
							background: INK,
							borderRadius: 10,
							fontFamily: MONO,
							fontSize: 17,
							color: PAPER,
						}}
					>
						<span style={{ color: "#a78bfa" }}>$</span>
						<span style={{ marginLeft: 12 }}>npm create @pylonsync/pylon@latest</span>
					</div>
				</div>

				<div style={{ width: 600, display: "flex", borderLeft: `1px solid ${RULE}` }}>
					<RuntimeBoard />
				</div>
			</div>
		</div>,
		{
			...size,
			fonts: [
				{ name: SANS, data: font("Geist-400.ttf"), weight: 400, style: "normal" },
				{ name: SANS, data: font("Geist-600.ttf"), weight: 600, style: "normal" },
				{ name: SANS, data: font("Geist-700.ttf"), weight: 700, style: "normal" },
				{ name: MONO, data: font("GeistMono-400.ttf"), weight: 400, style: "normal" },
			],
		},
	);
}
