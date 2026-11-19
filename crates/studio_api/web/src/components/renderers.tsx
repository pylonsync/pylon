import { createElement } from "react";
import { Check, ExternalLink } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { CopyButton } from "@/components/copy-button";
import {
	getExtensionRenderer,
	type CellRendererProps,
} from "@/lib/extensions";
import {
	formatBytes,
	formatDateTime,
	jsonPreview,
	parseDate,
	relativeTime,
	shortId,
} from "@/lib/format";
import { onLinkClick, rowUrl } from "@/lib/navigation";
import type {
	BadgeVariant,
	ColumnConfig,
	ColumnRenderer,
} from "@/lib/studio-config";
import { cn } from "@/lib/utils";

// ---------------------------------------------------------------------------
// Configured cells: a column with a `renderer` in studio.config.ts
// ---------------------------------------------------------------------------

export function CellRenderer({
	value,
	row,
	column,
}: CellRendererProps): React.ReactNode {
	const r = column.renderer;
	if (!r) return <TextCell value={value} />;

	switch (r.kind) {
		case "text":
			return <TextCell value={value} truncate={r.truncate} mono={r.mono} />;
		case "avatar":
			return <AvatarCell value={value} row={row} renderer={r} />;
		case "badge":
			return <BadgeCell value={value} renderer={r} />;
		case "date":
			return <DateCell value={value} format={r.format ?? "relative"} />;
		case "link":
			return <LinkCell value={value} row={row} renderer={r} />;
		case "boolean":
			return (
				<BoolCell
					value={value}
					trueLabel={r.trueLabel}
					falseLabel={r.falseLabel}
				/>
			);
		case "number":
			return <NumberCell value={value} renderer={r} />;
		case "json":
			return <JsonCell value={value} truncate={r.truncate} />;
		case "custom": {
			const custom = getExtensionRenderer(r.componentId);
			if (custom) return createElement(custom, { value, row, column });
			return <UnknownRenderer id={r.componentId} />;
		}
	}
}

// ---------------------------------------------------------------------------
// Automatic cells: a column with no renderer, shown by its manifest type
// ---------------------------------------------------------------------------

export interface AutoCellProps {
	value: unknown;
	/** The manifest type of the field, such as `string` or `id(Company)`. */
	type: string | undefined;
	/** The field is the row id. */
	isId?: boolean;
	/** The entity the value points to, when the field is a reference. */
	refEntity?: string | null;
	/** The display value of the row the value points to, when it is known. */
	refLabel?: string | null;
}

export function AutoCell({
	value,
	type,
	isId,
	refEntity,
	refLabel,
}: AutoCellProps): React.ReactNode {
	if (isEmpty(value)) return <Dash />;
	if (isId && typeof value === "string") return <IdCell id={value} />;
	if (refEntity && typeof value === "string") {
		return <RefCell entity={refEntity} id={value} label={refLabel} />;
	}
	switch (type) {
		case "bool":
			return <BoolCell value={value} />;
		case "datetime":
			return <DateCell value={value} format="datetime" />;
		case "int":
		case "float":
			return <NumberCell value={value} renderer={{ kind: "number" }} />;
		case "json":
			return <JsonCell value={value} />;
	}
	if (typeof value === "boolean") return <BoolCell value={value} />;
	if (typeof value === "number") {
		return <NumberCell value={value} renderer={{ kind: "number" }} />;
	}
	if (typeof value === "object") return <JsonCell value={value} />;
	return <TextCell value={value} />;
}

// ---------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------

function isEmpty(value: unknown): boolean {
	return value === null || value === undefined || value === "";
}

/** The mark for a cell with no value. */
export function Dash() {
	return (
		<span className="text-subtle-foreground" aria-label="No value">
			–
		</span>
	);
}

/**
 * An id: the first 8 characters in monospace. The full id is the tooltip, and
 * a copy button shows while the pointer is on the row or focus is in the cell.
 */
export function IdCell({ id, className }: { id: string; className?: string }) {
	return (
		<span className={cn("group/id inline-flex items-center gap-1", className)}>
			<span className="font-mono text-xs text-muted-foreground" title={id}>
				{shortId(id)}
			</span>
			<CopyButton
				value={id}
				label="Copy the id"
				className="opacity-0 group-hover/row:opacity-100 group-focus-within/id:opacity-100 focus-visible:opacity-100"
			/>
		</span>
	);
}

/** A reference to a row of another entity, as a link that opens that row. */
export function RefCell({
	entity,
	id,
	label,
}: {
	entity: string;
	id: string;
	label?: string | null;
}) {
	const url = rowUrl(entity, id);
	return (
		<a
			href={url}
			title={`${entity} ${id}`}
			className={cn(
				"text-link underline-offset-2 hover:underline",
				!label && "font-mono text-xs",
			)}
			onClick={(e) => {
				e.stopPropagation();
				onLinkClick(e, url);
			}}
		>
			{label ?? shortId(id)}
		</a>
	);
}

function TextCell({
	value,
	truncate,
	mono = false,
}: {
	value: unknown;
	truncate?: number;
	mono?: boolean;
}) {
	if (isEmpty(value)) return <Dash />;
	const full = typeof value === "string" ? value : jsonPreview(value);
	// The cell clips long text to one line with CSS. A configured `truncate`
	// also cuts the text to that many characters.
	const text =
		truncate && full.length > truncate ? `${full.slice(0, truncate)}…` : full;
	return (
		<span
			className={mono ? "font-mono text-xs" : undefined}
			title={full.length > 40 ? full : undefined}
		>
			{text}
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
	const initials =
		name
			.split(/\s+/)
			.filter(Boolean)
			.slice(0, 2)
			.map((p) => p.charAt(0).toUpperCase())
			.join("") || "?";

	return (
		<span className="inline-flex max-w-full items-center gap-2 align-middle">
			<span className="relative flex size-5 shrink-0 items-center justify-center overflow-hidden rounded-full bg-accent text-[9px] font-semibold">
				{image ? (
					<img
						src={image}
						alt=""
						className="size-full object-cover"
						onError={(e) => {
							e.currentTarget.style.display = "none";
						}}
					/>
				) : (
					initials
				)}
			</span>
			<span className="truncate font-medium">{name || <Dash />}</span>
			{subtitle && (
				<span className="truncate text-xs text-muted-foreground">{subtitle}</span>
			)}
		</span>
	);
}

function BadgeCell({
	value,
	renderer,
}: {
	value: unknown;
	renderer: Extract<ColumnRenderer, { kind: "badge" }>;
}) {
	if (isEmpty(value)) return <Dash />;
	const key = String(value);
	const variant: BadgeVariant =
		renderer.variants?.[key] ?? renderer.variants?.["*"] ?? "gray";
	return (
		<Badge variant={variant} dot={renderer.dot !== false}>
			{key}
		</Badge>
	);
}

function DateCell({
	value,
	format,
}: {
	value: unknown;
	format: "relative" | "absolute" | "iso" | "datetime";
}) {
	if (isEmpty(value)) return <Dash />;
	const d = parseDate(value);
	if (!d) return <TextCell value={value} mono />;
	const iso = d.toISOString();
	if (format === "iso") return <span className="font-mono text-xs">{iso}</span>;
	if (format === "relative") return <span title={iso}>{relativeTime(d)}</span>;
	if (format === "absolute") {
		return (
			<span title={iso}>
				{d.toLocaleDateString(undefined, {
					year: "numeric",
					month: "short",
					day: "numeric",
				})}
			</span>
		);
	}
	return <span title={iso}>{formatDateTime(d)}</span>;
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
	if (isEmpty(value)) return <Dash />;
	const href = interpolate(renderer.href, value, row);
	const external = renderer.external ?? /^https?:\/\//.test(href);
	return (
		<a
			href={href}
			target={external ? "_blank" : undefined}
			rel={external ? "noreferrer" : undefined}
			onClick={(e) => e.stopPropagation()}
			className="inline-flex items-center gap-1 text-link underline-offset-2 hover:underline"
		>
			{String(value)}
			{external && <ExternalLink className="size-3" />}
		</a>
	);
}

/** A check for true and a dash for false. Configured labels replace both. */
function BoolCell({
	value,
	trueLabel,
	falseLabel,
}: {
	value: unknown;
	trueLabel?: string;
	falseLabel?: string;
}) {
	if (value === null || value === undefined) return <Dash />;
	const truthy = !!value;
	if (trueLabel || falseLabel) {
		return <span>{truthy ? (trueLabel ?? "Yes") : (falseLabel ?? "No")}</span>;
	}
	return truthy ? (
		<Check className="inline size-3.5" aria-label="True" />
	) : (
		<span className="text-subtle-foreground" aria-label="False">
			–
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
	if (isEmpty(value)) return <Dash />;
	const n = typeof value === "number" ? value : Number(value);
	if (Number.isNaN(n)) return <TextCell value={value} />;
	const style = renderer.style ?? "decimal";
	if (style === "bytes") return <span>{formatBytes(n)}</span>;
	const opts: Intl.NumberFormatOptions = {};
	if (style === "currency") {
		opts.style = "currency";
		opts.currency = renderer.currency ?? "USD";
	} else if (style === "percent") {
		opts.style = "percent";
	}
	return <span>{new Intl.NumberFormat(renderer.locale, opts).format(n)}</span>;
}

function JsonCell({ value, truncate }: { value: unknown; truncate?: number }) {
	if (value === null || value === undefined) return <Dash />;
	const full = jsonPreview(value);
	const text =
		truncate && full.length > truncate ? `${full.slice(0, truncate)}…` : full;
	return (
		<span
			className="font-mono text-xs text-muted-foreground"
			title={full.length > 2000 ? `${full.slice(0, 2000)}…` : full}
		>
			{text}
		</span>
	);
}

function UnknownRenderer({ id }: { id: string }) {
	return (
		<span
			className="text-xs text-status-amber-fg"
			title={`The renderer "${id}" is not registered in studio.entry.tsx.`}
		>
			?{id}
		</span>
	);
}

function interpolate(
	template: string,
	value: unknown,
	row: Record<string, unknown>,
): string {
	return template
		.replace(/\{value\}/g, encodeURIComponent(String(value ?? "")))
		.replace(/\{row\.([\w.]+)\}/g, (_, path: string) => {
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

export type { ColumnConfig };
