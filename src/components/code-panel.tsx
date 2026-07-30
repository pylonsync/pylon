"use client";

// Shared code surface for the marketing site. Lives here rather than inside a
// page so the landing page, the product pages, and anything added later render
// code identically — the product pages previously used a separate dark-on-black
// block, which read as a different design system on the same site.
//
// Tokenizing is deliberately lightweight (strings, keywords, identifiers,
// punctuation) instead of pulling in a highlighter: marketing samples are short
// and hand-written, and a real grammar would cost more bundle than it earns.

const CODE_KEYWORDS = new Set([
	"const",
	"entity",
	"policy",
	"field",
	"true",
	"false",
	"null",
	"import",
	"export",
	"default",
	"await",
	"function",
	"return",
]);

export function highlightLine(line: string): React.ReactNode {
	if (line.trimStart().startsWith("//") || line.trimStart().startsWith("#")) {
		return <span className="text-[var(--color-ink-4)]">{line}</span>;
	}
	if (line === "") return " ";
	const parts = line.match(/("[^"]*")|([A-Za-z_$][\w$]*)|([^A-Za-z_$"]+)/g) ?? [
		line,
	];
	return parts.map((tok, i) => {
		if (tok.startsWith('"')) {
			return (
				<span key={i} className="text-[var(--color-status-live)]">
					{tok}
				</span>
			);
		}
		if (CODE_KEYWORDS.has(tok)) {
			return (
				<span key={i} className="text-[var(--color-cobalt)]">
					{tok}
				</span>
			);
		}
		if (/^[A-Za-z_$]/.test(tok)) {
			return (
				<span key={i} className="text-[var(--color-ink-2)]">
					{tok}
				</span>
			);
		}
		return (
			<span key={i} className="text-[var(--color-ink-3)]">
				{tok}
			</span>
		);
	});
}

// Shell samples get a cobalt `$` and no tokenizing — running the TypeScript
// keyword pass over a command line mis-colours ordinary words.
function renderLine(line: string, shell: boolean): React.ReactNode {
	if (!shell) return highlightLine(line);
	const trimmed = line.trimStart();
	if (!trimmed.startsWith("$")) {
		return <span className="text-[var(--color-ink-3)]">{line || " "}</span>;
	}
	const indent = line.slice(0, line.length - trimmed.length);
	return (
		<>
			{indent}
			<span className="text-[var(--color-cobalt)]">$</span>
			<span className="text-[var(--color-ink-2)]">{trimmed.slice(1)}</span>
		</>
	);
}

export function CodePanel({
	filename,
	code,
	shell = false,
	className,
}: {
	filename: string;
	code: string;
	shell?: boolean;
	className?: string;
}) {
	return (
		<div
			className={`w-full min-w-0 max-w-full overflow-hidden rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] shadow-[0_30px_60px_-40px_rgba(15,23,42,0.4)] ${className ?? ""}`}
		>
			<div className="flex items-center gap-2.5 border-b border-[var(--color-rule)] px-4 py-2.5">
				<span className="flex gap-1.5">
					<span className="size-2.5 rounded-full bg-[var(--color-rule)]" />
					<span className="size-2.5 rounded-full bg-[var(--color-rule)]" />
					<span className="size-2.5 rounded-full bg-[var(--color-rule)]" />
				</span>
				<span className="truncate font-mono text-[11px] text-[var(--color-ink-4)]">
					{filename}
				</span>
			</div>
			<pre className="overflow-x-auto px-5 py-4 font-mono text-[12.5px] leading-[1.7] text-[var(--color-ink-2)]">
				<code>
					{code.split("\n").map((line, i) => (
						// eslint-disable-next-line react/no-array-index-key
						<div key={i}>{renderLine(line, shell)}</div>
					))}
				</code>
			</pre>
		</div>
	);
}
