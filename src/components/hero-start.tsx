"use client";

import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { useCopy } from "../lib/use-copy";

// The hero's only action, in two modes.
//
// A single `npm create` pill assumed the reader was going to type it. The page
// claims agents ship the apps, so the agent path deserves equal billing: one
// paste that hands over the skill, the docs index, the scaffold command, and
// the runtime requirement, instead of making someone assemble it from four
// pages.
//
// Every URL and command below is real and load-bearing. Check them against
// apps/pylonsync-site/public/pylon-skill.md before editing — that file is the
// same contract, and the two drifting apart is how an agent gets sent to a
// 404.

const SCAFFOLD = "npm create @pylonsync/pylon@latest";

const AGENT_BRIEF = `Build an app with Pylon (pylonsync.com), an agent-native full-stack
framework: schema, row-level policies, server functions, live queries,
and server-rendered React on one port.

Load the skill first:
  npx skills add pylonsync/pylon

References:
  Docs index (agent-readable)  https://docs.pylonsync.com/llms.txt
  Raw skill file               https://www.pylonsync.com/pylon-skill.md
  SDK source                   https://github.com/pylonsync/pylon/tree/main/packages
  Example apps                 https://github.com/pylonsync/pylon/tree/main/examples

Scaffold:
  npm create @pylonsync/pylon@latest my-app
  # add --template saas | chat | shop | directory | crm | ai-chat (18 total)

Requires Bun >= 1.0 on PATH.

Then build: `;

// Terminal surface. Fixed hexes, not theme tokens, on purpose: this panel is
// dark in BOTH themes, and `var(--color-ink)` flips to near-white under .dark —
// which would paint a white block and white-on-white text. A terminal is a
// terminal in either theme.
const TERM = {
	bg: "#18181b",
	bgHeader: "#141416",
	rule: "#27272a",
	prose: "#a1a1aa",
	label: "#fafafa",
	command: "#34d399",
	url: "#a78bfa",
	muted: "#71717a",
} as const;

const URL_RE = /https?:\/\/[^\s]+/;

/**
 * Semantic highlighting for the brief. It is prose, commands, and URLs rather
 * than one language, so this colours by role instead of by grammar:
 * section labels, runnable commands, links, and `#` comments.
 *
 * Operates on the same `AGENT_BRIEF` string the copy button sends, so what the
 * reader sees and what lands in their agent can't diverge.
 */
function highlightBrief(line: string): React.ReactNode {
	if (line === "") return " ";

	const trimmed = line.trimStart();
	const indent = line.slice(0, line.length - trimmed.length);

	if (trimmed.startsWith("#")) {
		return <span style={{ color: TERM.muted }}>{line}</span>;
	}

	// A reference row: "  Label   https://…". Colour the URL, mute the label.
	const url = trimmed.match(URL_RE);
	if (url) {
		const before = line.slice(0, line.indexOf(url[0]));
		const after = line.slice(line.indexOf(url[0]) + url[0].length);
		return (
			<>
				<span style={{ color: TERM.prose }}>{before}</span>
				<span style={{ color: TERM.url }}>{url[0]}</span>
				<span style={{ color: TERM.prose }}>{after}</span>
			</>
		);
	}

	// Indented lines are things to run.
	if (indent.length > 0) {
		return (
			<>
				{indent}
				<span style={{ color: TERM.command }}>{trimmed}</span>
			</>
		);
	}

	// A bare line ending in ":" heads a section.
	if (trimmed.endsWith(":")) {
		return <span style={{ color: TERM.label }}>{line}</span>;
	}

	return <span style={{ color: TERM.prose }}>{line}</span>;
}

type Mode = "self" | "agent";

export function HeroStart() {
	const [mode, setMode] = useState<Mode>("self");

	return (
		<div className="mx-auto mt-9 w-full max-w-[640px]">
			{/* Two peers, not a primary and a fallback. The agent path is the one
			    the headline is about, so it is not demoted to a footnote. */}
			<div
				role="tablist"
				aria-label="How do you want to start?"
				className="flex items-center justify-center gap-1"
			>
				<ModeTab
					id="self"
					label="Run it yourself"
					active={mode === "self"}
					onSelect={setMode}
				/>
				<ModeTab
					id="agent"
					label="Hand it to your agent"
					active={mode === "agent"}
					onSelect={setMode}
				/>
			</div>

			<div className="mt-4">
				{mode === "self" ? <SelfPanel /> : <AgentPanel />}
			</div>
		</div>
	);
}

function ModeTab({
	id,
	label,
	active,
	onSelect,
}: {
	id: Mode;
	label: string;
	active: boolean;
	onSelect: (m: Mode) => void;
}) {
	return (
		<button
			type="button"
			role="tab"
			aria-selected={active}
			onClick={() => onSelect(id)}
			className={`rounded-full px-3 py-1.5 text-[13px] transition-colors ${
				active
					? "bg-[var(--color-brand-soft)] text-[var(--color-brand)]"
					: "text-[var(--color-ink-3)] hover:text-[var(--color-ink)]"
			}`}
		>
			{label}
		</button>
	);
}

function SelfPanel() {
	return (
		<div className="flex flex-col items-center gap-2.5">
			<InstallCommand command={SCAFFOLD} />
			<p className="text-[12.5px] text-[var(--color-ink-3)]">
				Add{" "}
				<code className="font-mono text-[var(--color-ink-2)]">
					--template saas
				</code>{" "}
				to start from one of 18 working apps.
			</p>
		</div>
	);
}

function AgentPanel() {
	const { copied, copy } = useCopy();
	return (
		<div
			className="overflow-hidden rounded-[var(--radius-lg)] border text-left shadow-[var(--shadow-card-hover)]"
			style={{ background: TERM.bg, borderColor: TERM.rule }}
		>
			<div
				className="flex items-center justify-between border-b px-3.5 py-2"
				style={{ background: TERM.bgHeader, borderColor: TERM.rule }}
			>
				<span
					className="font-mono text-[10.5px] uppercase tracking-[0.14em]"
					style={{ color: TERM.muted }}
				>
					Paste into Claude Code, Codex, or Cursor
				</span>
				<button
					type="button"
					onClick={() => copy(AGENT_BRIEF)}
					aria-label="Copy the agent brief"
					className="inline-flex items-center gap-1.5 rounded-[var(--radius-sm)] px-2 py-1 text-[12px] transition-colors hover:brightness-150"
					style={{ color: copied ? TERM.command : TERM.prose }}
				>
					{copied ? (
						<>
							<Check className="size-3.5" />
							Copied
						</>
					) : (
						<>
							<Copy className="size-3.5" />
							Copy
						</>
					)}
				</button>
			</div>
			{/* Capped and scrollable: the brief is deliberately longer than fits,
			    and letting it set the hero's height pushed the diagram off-screen. */}
			<pre className="max-h-[168px] overflow-auto px-3.5 py-3 font-mono text-[11px] leading-[1.65]">
				<code>
					{AGENT_BRIEF.split("\n").map((line, i) => (
						// Order is fixed for the lifetime of the component, so the index
						// is a stable key here.
						// biome-ignore lint/suspicious/noArrayIndexKey: static line list
						<span key={i} className="block">
							{highlightBrief(line)}
						</span>
					))}
				</code>
			</pre>
		</div>
	);
}

export function InstallCommand({ command }: { command: string }) {
	const { copied, copy } = useCopy();
	return (
		<button
			type="button"
			onClick={() => copy(command)}
			aria-label={`Copy: ${command}`}
			// max-w-full + nowrap: on a 390px screen the command wrapped inside the
			// pill, which centred the two halves under a vertically-centred `$` and
			// stopped looking like a command line. It sheds a point of type below sm
			// instead, where it fits on one line.
			//
			// Dark in both themes, like the agent panel — see the TERM note. A pale
			// pill read as a form field; this one reads as a terminal.
			className="group inline-flex max-w-full items-center gap-3 rounded-full border py-3 pl-5 pr-4 font-mono text-[12px] shadow-[var(--shadow-card-hover)] transition-[border-color] sm:text-[13px]"
			style={{
				background: TERM.bg,
				borderColor: TERM.rule,
				color: "#e4e4e7",
			}}
		>
			<span className="select-none" style={{ color: TERM.url }}>
				$
			</span>
			<span className="truncate tracking-tight">{command}</span>
			<span
				className="ml-1 transition-opacity group-hover:opacity-100"
				style={{ color: copied ? TERM.command : TERM.muted, opacity: 0.9 }}
			>
				{copied ? <Check className="size-4" /> : <Copy className="size-4" />}
			</span>
		</button>
	);
}
