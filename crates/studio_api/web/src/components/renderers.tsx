import { ExternalLink } from "lucide-react";
import type {
	BadgeVariant,
	ColumnConfig,
	ColumnRenderer,
} from "@/lib/studio-config";
import {
	getExtensionRenderer,
	type CellRendererProps,
} from "@/lib/extensions";

// ---------------------------------------------------------------------------
// Cell renderer dispatcher
// ---------------------------------------------------------------------------

export function CellRenderer({
	value,
	row,
	column,
}: CellRendererProps): React.ReactNode {
	const r = column.renderer;
	if (!r) return <TextCell value={value} truncate={80} />;

	switch (r.kind) {
		case "text":
			return <TextCell value={value} truncate={r.truncate} mono={r.mono} />;
		case "avatar":
			return <AvatarCell value={value} row={row} renderer={r} />;
		case "badge":
			return <BadgeCell value={value} renderer={r} />;
		case "date":
			return <DateCell value={value} renderer={r} />;
		case "link":
			return <LinkCell value={value} row={row} renderer={r} />;
		case "boolean":
			return <BoolCell value={value} renderer={r} />;
		case "number":
			return <NumberCell value={value} renderer={r} />;
		case "json":
			return <JsonCell value={value} truncate={r.truncate ?? 60} />;
		case "custom": {
			const Custom = getExtensionRenderer(r.componentId);
			if (Custom) return <Custom value={value} row={row} column={column} />;
			return <UnknownRenderer id={r.componentId} />;
		}
	}
}

// ---------------------------------------------------------------------------
// Built-in renderers
// ---------------------------------------------------------------------------

function TextCell({
	value,
	truncate = 80,
	mono = false,
}: {
	value: unknown;
	truncate?: number;
	mono?: boolean;
}) {
	if (value === null || value === undefined)
		return <span className="text-muted-foreground">—</span>;
	let str = typeof value === "string" ? value : String(value);
	let truncated = false;
	if (str.length > truncate) {
		str = str.slice(0, truncate) + "…";
		truncated = true;
	}
	return (
		<span
			className={mono ? "font-mono text-xs" : "text-sm"}
			title={truncated ? String(value) : undefined}
		>
			{str}
		</span>
	);
}

function AvatarCell({
	value,
	row,
	renderer,
}: {
	value: unknown;
	row: Record<string, unknown>;
	renderer: Extract<ColumnRenderer, { kind: "avatar" }>;
}) {
	const name = String(
		(renderer.nameField ? row[renderer.nameField] : value) ?? "",
	);
	const image = renderer.imageField
		? String(row[renderer.imageField] ?? "")
		: "";
	const subtitle = renderer.subtitleField
		? String(row[renderer.subtitleField] ?? "")
		: "";
	const initials = name
		.split(/\s+/)
		.filter(Boolean)
		.slice(0, 2)
		.map((p) => p.charAt(0).toUpperCase())
		.join("") || "?";

	return (
		<div className="flex items-center gap-3">
			<div className="relative flex size-8 items-center justify-center overflow-hidden rounded-full bg-secondary text-xs font-semibold text-secondary-foreground shrink-0">
				{image ? (
					<img
						src={image}
						alt=""
						className="size-full object-cover"
						onError={(e) => {
							(e.currentTarget as HTMLImageElement).style.display = "none";
						}}
					/>
				) : (
					<span>{initials}</span>
				)}
			</div>
			<div className="flex flex-col leading-tight min-w-0">
				<span className="text-sm font-medium truncate">{name || "—"}</span>
				{subtitle && (
					<span className="text-xs text-muted-foreground truncate">
						{subtitle}
					</span>
				)}
			</div>
		</div>
	);
}

const BADGE_CLASSES: Record<BadgeVariant, string> = {
	green: "bg-status-green-bg text-status-green-fg",
	red: "bg-status-red-bg text-status-red-fg",
	amber: "bg-status-amber-bg text-status-amber-fg",
	blue: "bg-status-blue-bg text-status-blue-fg",
	gray: "bg-status-gray-bg text-status-gray-fg",
};
const DOT_CLASSES: Record<BadgeVariant, string> = {
	green: "bg-status-green-fg",
	red: "bg-status-red-fg",
	amber: "bg-status-amber-fg",
	blue: "bg-status-blue-fg",
	gray: "bg-status-gray-fg",
};

function BadgeCell({
	value,
	renderer,
}: {
	value: unknown;
	renderer: Extract<ColumnRenderer, { kind: "badge" }>;
}) {
	if (value === null || value === undefined)
		return <span className="text-muted-foreground">—</span>;
	const key = String(value);
	const variant: BadgeVariant =
		renderer.variants?.[key] ?? renderer.variants?.["*"] ?? "gray";
	const dot = renderer.dot !== false;
	return (
		<span
			className={`inline-flex items-center gap-1.5 rounded-md px-2 py-0.5 text-xs font-medium ${BADGE_CLASSES[variant]}`}
		>
			{dot && <span className={`size-1.5 rounded-full ${DOT_CLASSES[variant]}`} />}
			{key}
		</span>
	);
}

function DateCell({
	value,
	renderer,
}: {
	value: unknown;
	renderer: Extract<ColumnRenderer, { kind: "date" }>;
}) {
	if (value === null || value === undefined)
		return <span className="text-muted-foreground">—</span>;
	const d = parseDate(value);
	if (!d) return <TextCell value={value} truncate={40} mono />;
	const fmt = renderer.format ?? "relative";
	if (fmt === "iso") return <span className="text-xs font-mono">{d.toISOString()}</span>;
	if (fmt === "absolute")
		return (
			<span className="text-sm" title={d.toISOString()}>
				{d.toLocaleDateString(undefined, {
					year: "numeric",
					month: "short",
					day: "numeric",
				})}
			</span>
		);
	return (
		<span className="text-sm" title={d.toLocaleString()}>
			{relativeTime(d)}
		</span>
	);
}

function LinkCell({
	value,
	row,
	renderer,
}: {
	value: unknown;
	row: Record<string, unknown>;
	renderer: Extract<ColumnRenderer, { kind: "link" }>;
}) {
	if (value === null || value === undefined)
		return <span className="text-muted-foreground">—</span>;
	const href = interpolate(renderer.href, value, row);
	const external = renderer.external ?? /^https?:\/\//.test(href);
	return (
		<a
			href={href}
			target={external ? "_blank" : undefined}
			rel={external ? "noreferrer" : undefined}
			onClick={(e) => e.stopPropagation()}
			className="inline-flex items-center gap-1 text-sm text-primary hover:underline"
		>
			{String(value)}
			{external && <ExternalLink className="size-3" />}
		</a>
	);
}

function BoolCell({
	value,
	renderer,
}: {
	value: unknown;
	renderer: Extract<ColumnRenderer, { kind: "boolean" }>;
}) {
	if (value === null || value === undefined)
		return <span className="text-muted-foreground">—</span>;
	const truthy = !!value;
	return (
		<span className="text-sm">
			{truthy ? renderer.trueLabel ?? "Yes" : renderer.falseLabel ?? "No"}
		</span>
	);
}

function NumberCell({
	value,
	renderer,
}: {
	value: unknown;
	renderer: Extract<ColumnRenderer, { kind: "number" }>;
}) {
	if (value === null || value === undefined)
		return <span className="text-muted-foreground">—</span>;
	const n = typeof value === "number" ? value : Number(value);
	if (Number.isNaN(n)) return <TextCell value={value} truncate={40} />;
	const style = renderer.style ?? "decimal";
	if (style === "bytes") return <span className="text-sm tabular-nums">{formatBytes(n)}</span>;
	const opts: Intl.NumberFormatOptions = {};
	if (style === "currency") {
		opts.style = "currency";
		opts.currency = renderer.currency ?? "USD";
	} else if (style === "percent") {
		opts.style = "percent";
	}
	const fmt = new Intl.NumberFormat(renderer.locale, opts);
	return <span className="text-sm tabular-nums">{fmt.format(n)}</span>;
}

function JsonCell({ value, truncate }: { value: unknown; truncate: number }) {
	if (value === null || value === undefined)
		return <span className="text-muted-foreground">—</span>;
	let s: string;
	try {
		s = JSON.stringify(value);
	} catch {
		s = String(value);
	}
	const trunc = s.length > truncate ? `${s.slice(0, truncate)}…` : s;
	return (
		<span className="font-mono text-xs text-muted-foreground" title={s}>
			{trunc}
		</span>
	);
}

function UnknownRenderer({ id }: { id: string }) {
	return (
		<span className="text-xs text-status-amber-fg" title={`Renderer "${id}" not registered`}>
			?{id}
		</span>
	);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function parseDate(v: unknown): Date | null {
	if (v instanceof Date) return v;
	if (typeof v === "number") return new Date(v);
	if (typeof v === "string") {
		const d = new Date(v);
		return Number.isNaN(d.getTime()) ? null : d;
	}
	return null;
}

function relativeTime(d: Date): string {
	const now = Date.now();
	const delta = (now - d.getTime()) / 1000;
	const past = delta >= 0;
	const abs = Math.abs(delta);
	const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
	const sign = past ? -1 : 1;
	if (abs < 60) return rtf.format(sign * Math.round(abs), "second");
	if (abs < 3600) return rtf.format(sign * Math.round(abs / 60), "minute");
	if (abs < 86_400) return rtf.format(sign * Math.round(abs / 3600), "hour");
	if (abs < 86_400 * 30) return rtf.format(sign * Math.round(abs / 86_400), "day");
	if (abs < 86_400 * 365)
		return rtf.format(sign * Math.round(abs / (86_400 * 30)), "month");
	return rtf.format(sign * Math.round(abs / (86_400 * 365)), "year");
}

function formatBytes(n: number): string {
	const units = ["B", "KB", "MB", "GB", "TB", "PB"];
	let u = 0;
	let v = n;
	while (v >= 1024 && u < units.length - 1) {
		v /= 1024;
		u++;
	}
	return `${v.toFixed(u === 0 ? 0 : 1)} ${units[u]}`;
}

function interpolate(
	template: string,
	value: unknown,
	row: Record<string, unknown>,
): string {
	return template
		.replace(/\{value\}/g, encodeURIComponent(String(value ?? "")))
		.replace(/\{row\.([\w.]+)\}/g, (_, path) => {
			let v: unknown = row;
			for (const seg of path.split(".")) {
				if (v && typeof v === "object" && seg in (v as Record<string, unknown>)) {
					v = (v as Record<string, unknown>)[seg];
				} else {
					return "";
				}
			}
			return encodeURIComponent(String(v ?? ""));
		});
}

// Re-export for ResourceList
export type { ColumnConfig };
