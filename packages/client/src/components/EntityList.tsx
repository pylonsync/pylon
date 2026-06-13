"use client";
import React from "react";

import { type ReactNode, useMemo } from "react";
import { db, type Row } from "@pylonsync/react";
import { cn } from "../lib/cn";

export interface EntityListColumn<T extends Row = Row> {
	/** Field key on the row. */
	field: keyof T & string;
	/** Column header. Defaults to a humanized `field` name. */
	label?: ReactNode;
	/** Custom cell renderer. Default: stringified value. */
	render?: (value: unknown, row: T) => ReactNode;
	/** Tailwind / utility classes applied to the `<td>`. */
	className?: string;
}

export interface EntityListProps<T extends Row = Row> {
	/** Entity name (matches your schema declaration). */
	entity: string;
	/** Columns. When omitted, infers from the first row's keys. */
	columns?: Array<EntityListColumn<T> | string>;
	/** Live filter clause passed straight to `db.useQuery`. */
	where?: Record<string, unknown>;
	/** Server-side ordering. */
	orderBy?: Record<string, "asc" | "desc">;
	/** Max rows. */
	limit?: number;
	/** Click handler — gets the row. */
	onRowClick?: (row: T) => void;
	/** Render when there are zero rows. */
	emptyState?: ReactNode;
	/** Optional caption shown above the table (e.g., "Posts"). */
	title?: ReactNode;
	/** Render arbitrary actions in the top-right (e.g., a "+ New" button). */
	actions?: ReactNode;
	className?: string;
}

/**
 * Reactive table for any entity. Wraps `db.useQuery` so rows re-render
 * live when the underlying data changes (insert / update / delete from
 * any peer). Columns are explicit; inference is a fallback for "just
 * give me something" demos.
 *
 * ```tsx
 * <EntityList
 *   entity="Post"
 *   columns={["title", "author", { field: "createdAt", render: relative }]}
 *   where={{ status: "published" }}
 *   onRowClick={(post) => router.push(`/posts/${post.id}`)}
 * />
 * ```
 */
export function EntityList<T extends Row = Row>({
	entity,
	columns,
	where,
	orderBy,
	limit,
	onRowClick,
	emptyState,
	title,
	actions,
	className,
}: EntityListProps<T>) {
	const { data, loading, error } = db.useQuery<T>(entity, {
		where: where as Record<string, unknown> | undefined,
		orderBy,
		limit,
	});

	const resolvedColumns = useMemo(
		() => resolveColumns<T>(columns, data),
		[columns, data],
	);

	return (
		<div
			className={cn(
				"w-full overflow-hidden rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] shadow-sm",
				className,
			)}
		>
			{title || actions ? (
				<div className="flex items-center justify-between gap-3 border-b border-[var(--pylon-rule,#e5e7eb)] px-4 py-2.5">
					<p className="text-sm font-semibold tracking-tight text-[var(--pylon-ink,#0a0a0a)]">
						{title}
					</p>
					{actions ? <div className="flex gap-2">{actions}</div> : null}
				</div>
			) : null}
			{error ? (
				<p className="px-4 py-3 text-xs text-[var(--pylon-error-ink,#b91c1c)]">
					{error.message}
				</p>
			) : null}
			<div className="overflow-x-auto">
				<table className="w-full border-collapse text-left text-sm">
					<thead>
						<tr className="border-b border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper-2,#f4f4f5)]">
							{resolvedColumns.map((c) => (
								<th
									key={c.field}
									className="px-4 py-2 text-[11px] font-medium uppercase tracking-wider text-[var(--pylon-ink-2,#52525b)]"
								>
									{c.label ?? humanize(c.field)}
								</th>
							))}
						</tr>
					</thead>
					<tbody>
						{loading && data.length === 0 ? (
							<tr>
								<td
									colSpan={resolvedColumns.length || 1}
									className="px-4 py-6 text-center text-xs text-[var(--pylon-ink-3,#71717a)]"
								>
									Loading…
								</td>
							</tr>
						) : data.length === 0 ? (
							<tr>
								<td
									colSpan={resolvedColumns.length || 1}
									className="px-4 py-6 text-center text-xs text-[var(--pylon-ink-3,#71717a)]"
								>
									{emptyState ?? "No rows yet."}
								</td>
							</tr>
						) : (
							data.map((row, i) => (
								<tr
									key={(row as { id?: string }).id ?? i}
									onClick={
										onRowClick ? () => onRowClick(row) : undefined
									}
									className={cn(
										"border-b border-[var(--pylon-rule,#e5e7eb)] last:border-b-0",
										onRowClick &&
											"cursor-pointer transition-colors hover:bg-[var(--pylon-paper-2,#f4f4f5)]",
									)}
								>
									{resolvedColumns.map((c) => (
										<td
											key={c.field}
											className={cn(
												"px-4 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)]",
												c.className,
											)}
										>
											{c.render
												? c.render((row as Row)[c.field], row)
												: formatCell((row as Row)[c.field])}
										</td>
									))}
								</tr>
							))
						)}
					</tbody>
				</table>
			</div>
		</div>
	);
}

function resolveColumns<T extends Row>(
	cols: Array<EntityListColumn<T> | string> | undefined,
	rows: T[],
): EntityListColumn<T>[] {
	if (cols && cols.length > 0) {
		return cols.map((c) =>
			typeof c === "string"
				? ({ field: c as keyof T & string } satisfies EntityListColumn<T>)
				: c,
		);
	}
	const first = rows[0];
	if (!first) return [];
	return Object.keys(first)
		.filter((k) => !k.startsWith("_"))
		.slice(0, 6)
		.map((k) => ({ field: k as keyof T & string }));
}

function humanize(field: string): string {
	return field
		.replace(/([A-Z])/g, " $1")
		.replace(/^./, (c) => c.toUpperCase())
		.replace(/_/g, " ")
		.trim();
}

function formatCell(value: unknown): ReactNode {
	if (value === null || value === undefined) {
		return <span className="text-[var(--pylon-ink-3,#a1a1aa)]">—</span>;
	}
	if (typeof value === "boolean") return value ? "yes" : "no";
	if (value instanceof Date) return value.toLocaleString();
	if (typeof value === "object") return JSON.stringify(value);
	return String(value);
}
