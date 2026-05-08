import { useCallback, useEffect, useMemo, useState } from "react";
import {
	ArrowDown,
	ArrowUp,
	ChevronsUpDown,
	Filter as FilterIcon,
	Loader2,
	MoreHorizontal,
	Plus,
	RefreshCw,
	Search,
	Trash2,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { RowEditor } from "@/components/RowEditor";
import { CellRenderer } from "@/components/renderers";
import { ApiError, MANIFEST, type ManifestEntity, api } from "@/lib/pylon";
import type {
	BulkAction,
	ColumnConfig,
	RowAction,
	StudioConfig,
} from "@/lib/studio-config";

type Row = Record<string, unknown> & { id?: string };

const DEFAULT_PAGE_SIZES = [10, 25, 50, 100];

/// Server response shape. The /api/entities/{Name} endpoint may return
/// either a bare array (legacy / small tables) or an envelope with
/// total/page/per_page; we accept both. The web shell does its own
/// client-side paging when the server gives us the full array.
type ListResponse =
	| Row[]
	| {
			data: Row[];
			total?: number;
			page?: number;
			per_page?: number;
	  };

export function ResourceListPage({
	entity,
	config,
}: {
	entity: string;
	config: StudioConfig;
}) {
	const manifestEntity = MANIFEST.entities.find((e) => e.name === entity);
	const resourceCfg = config.resources?.[entity];
	const listCfg = resourceCfg?.list;

	const columns = useMemo<ColumnConfig[]>(
		() => resolveColumns(manifestEntity, listCfg),
		[manifestEntity, listCfg],
	);
	const visibleColumns = columns.filter((c) => !c.hidden);
	const pluralLabel =
		resourceCfg?.pluralLabel ??
		resourceCfg?.label ??
		`${entity}${entity.endsWith("s") ? "" : "s"}`;

	const pageSizes = listCfg?.pageSizes ?? DEFAULT_PAGE_SIZES;
	const [pageSize, setPageSize] = useState(
		listCfg?.defaultPageSize ?? pageSizes[0],
	);
	const [page, setPage] = useState(1);
	const [search, setSearch] = useState("");
	const [sort, setSort] = useState<{ field: string; order: "asc" | "desc" } | null>(
		listCfg?.defaultSort ?? null,
	);
	const [filters, setFilters] = useState<Record<string, string>>({});

	const [rows, setRows] = useState<Row[]>([]);
	const [total, setTotal] = useState<number | null>(null);
	const [serverPaged, setServerPaged] = useState(false);
	const [loading, setLoading] = useState(false);
	const [selected, setSelected] = useState<Set<string>>(new Set());
	const [insertOpen, setInsertOpen] = useState(false);
	const [insertJson, setInsertJson] = useState("{}");
	const [inspectRow, setInspectRow] = useState<Row | null>(null);

	const load = useCallback(async () => {
		setLoading(true);
		try {
			const params = new URLSearchParams();
			params.set("page", String(page));
			params.set("per_page", String(pageSize));
			if (sort) {
				params.set("sort", sort.field);
				params.set("order", sort.order);
			}
			if (search.trim()) params.set("q", search.trim());
			for (const [k, v] of Object.entries(filters)) {
				if (v) params.set(`filter[${k}]`, v);
			}
			const data = await api<ListResponse>(
				`/api/entities/${entity}?${params.toString()}`,
			);
			if (Array.isArray(data)) {
				// Legacy / non-paginated server. Apply client-side paging.
				setServerPaged(false);
				const filtered = clientFilter(data, search, filters, columns);
				const sorted = sort ? clientSort(filtered, sort) : filtered;
				setTotal(sorted.length);
				setRows(
					sorted.slice((page - 1) * pageSize, page * pageSize),
				);
			} else {
				setServerPaged(true);
				setRows(data.data ?? []);
				setTotal(data.total ?? null);
			}
		} catch (err) {
			if (err instanceof ApiError) toast.error(`${err.code}: ${err.message}`);
			else toast.error(err instanceof Error ? err.message : String(err));
			setRows([]);
			setTotal(0);
		} finally {
			setLoading(false);
		}
	}, [entity, page, pageSize, sort, search, filters, columns]);

	useEffect(() => {
		void load();
		setSelected(new Set());
	}, [load]);

	useEffect(() => {
		setPage(1);
		setSelected(new Set());
	}, [entity, search, filters, sort, pageSize]);

	const onDelete = async (row: Row) => {
		if (!row.id) return;
		try {
			await api(`/api/entities/${entity}/${row.id}`, { method: "DELETE" });
			toast.success(`Deleted ${row.id}`);
			void load();
		} catch (err) {
			if (err instanceof ApiError) toast.error(`${err.code}: ${err.message}`);
			else toast.error(err instanceof Error ? err.message : String(err));
		}
	};

	const onInsert = async () => {
		try {
			const data = JSON.parse(insertJson);
			await api(`/api/entities/${entity}`, {
				method: "POST",
				body: JSON.stringify(data),
			});
			toast.success(`Inserted into ${entity}`);
			setInsertOpen(false);
			setInsertJson("{}");
			void load();
		} catch (err) {
			if (err instanceof ApiError) toast.error(`${err.code}: ${err.message}`);
			else toast.error(err instanceof Error ? err.message : String(err));
		}
	};

	// Bulk-confirm dialog state. Destructive actions (anything that
	// would call DELETE) flow through this rather than firing on the
	// click; the prior `window.confirm()` fallback wasn't even invoked
	// for the default delete path because no `confirm` string was set.
	// Result: clicking Actions deleted everything you'd selected with
	// zero ack. Now Actions always opens this dialog for destructive
	// kinds; non-destructive (export, custom) run inline.
	const [pendingBulk, setPendingBulk] = useState<BulkAction | null>(null);

	const isDestructive = (action: BulkAction): boolean =>
		action.kind === "delete" || action.id === "delete";

	const onBulk = (action: BulkAction): void => {
		const ids = Array.from(selected);
		if (ids.length === 0) return;
		if (isDestructive(action)) {
			setPendingBulk(action);
			return;
		}
		void runBulk(action);
	};

	const runBulk = async (action: BulkAction): Promise<void> => {
		const ids = Array.from(selected);
		if (ids.length === 0) return;

		if (action.kind === "delete" || action.id === "delete") {
			let ok = 0;
			for (const id of ids) {
				try {
					await api(`/api/entities/${entity}/${id}`, { method: "DELETE" });
					ok++;
				} catch {
					/* surface aggregate count below */
				}
			}
			toast.success(`Deleted ${ok}/${ids.length} rows`);
			setSelected(new Set());
			void load();
			return;
		}
		if (action.kind === "export" || action.id === "export") {
			const subset = rows.filter((r) => selected.has(String(r.id ?? "")));
			const blob = new Blob([JSON.stringify(subset, null, 2)], {
				type: "application/json",
			});
			const url = URL.createObjectURL(blob);
			const a = document.createElement("a");
			a.href = url;
			a.download = `${entity}-export.json`;
			a.click();
			URL.revokeObjectURL(url);
			return;
		}
		toast.info(`Custom action "${action.id}" — wire via studio.entry.tsx`);
	};

	const totalPages = total !== null ? Math.max(1, Math.ceil(total / pageSize)) : 1;
	const recordsLabel = useMemo(() => {
		if (total === null) return `${rows.length} record${rows.length === 1 ? "" : "s"}`;
		const suffix = serverPaged
			? "server-side pagination, sorting & filtering"
			: "client-side pagination, sorting & filtering";
		return `${total} record${total === 1 ? "" : "s"} — ${suffix}`;
	}, [total, rows.length, serverPaged]);

	if (!manifestEntity) {
		return (
			<div className="rounded-lg border bg-card p-8 text-center">
				<h2 className="text-lg font-semibold">Unknown entity</h2>
				<p className="mt-1 text-sm text-muted-foreground">
					Entity <code className="font-mono">{entity}</code> isn't declared
					in the manifest.
				</p>
			</div>
		);
	}

	const allSelected =
		rows.length > 0 &&
		rows.every((r) => r.id && selected.has(String(r.id)));

	const filterableColumns = visibleColumns.filter((c) => c.filterable);
	const showFilters =
		(listCfg?.filterable ?? filterableColumns.length > 0) &&
		filterableColumns.length > 0;
	const showSearch =
		listCfg?.searchable ?? visibleColumns.some((c) => c.searchable);

	return (
		<div className="space-y-6">
			<div>
				<h1 className="text-2xl font-semibold tracking-tight">{pluralLabel}</h1>
				<p className="mt-1 text-sm text-muted-foreground">{recordsLabel}</p>
			</div>

			<div className="flex flex-wrap items-center gap-2">
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<Button
							size="sm"
							disabled={selected.size === 0}
							variant="default"
						>
							Actions ({selected.size})
							<ChevronsUpDown className="size-3.5 opacity-70" />
						</Button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="start">
						{(listCfg?.bulkActions ?? []).map((a) => (
							<DropdownMenuItem
								key={a.id}
								onClick={() => onBulk(a)}
								disabled={selected.size === 0}
								className={
									a.kind === "delete" || a.id === "delete"
										? "text-destructive focus:text-destructive"
										: undefined
								}
							>
								{a.label}
							</DropdownMenuItem>
						))}
						{(listCfg?.bulkActions ?? []).length > 0 && (
							<DropdownMenuSeparator />
						)}
						<DropdownMenuItem
							onClick={() =>
								onBulk({ id: "export", label: "Export JSON", kind: "export" })
							}
							disabled={selected.size === 0}
						>
							Export JSON
						</DropdownMenuItem>
						<DropdownMenuItem
							onClick={() =>
								onBulk({ id: "delete", label: "Delete", kind: "delete" })
							}
							disabled={selected.size === 0}
							className="text-destructive focus:text-destructive"
						>
							Delete…
						</DropdownMenuItem>
					</DropdownMenuContent>
				</DropdownMenu>

				{showSearch && (
					<div className="relative">
						<Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
						<Input
							value={search}
							onChange={(e) => setSearch(e.target.value)}
							placeholder={`Search ${pluralLabel.toLowerCase()}...`}
							className="h-9 w-72 pl-8"
						/>
					</div>
				)}

				{showFilters && (
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button size="sm" variant="outline">
								<FilterIcon className="size-3.5" />
								Filters
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent className="w-72">
							<DropdownMenuLabel>Filters</DropdownMenuLabel>
							<DropdownMenuSeparator />
							{filterableColumns.map((col) => (
								<div key={col.field} className="px-2 py-1.5 space-y-1">
									<Label className="text-xs">{labelFor(col)}</Label>
									{typeof col.filterable === "object" &&
									col.filterable.options ? (
										<Select
											value={filters[col.field] ?? ""}
											onValueChange={(v) =>
												setFilters((f) => ({ ...f, [col.field]: v }))
											}
										>
											<SelectTrigger className="h-8">
												<SelectValue placeholder="Any" />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="">Any</SelectItem>
												{col.filterable.options.map((o) => (
													<SelectItem
														key={String(o.value)}
														value={String(o.value)}
													>
														{o.label}
													</SelectItem>
												))}
											</SelectContent>
										</Select>
									) : (
										<Input
											value={filters[col.field] ?? ""}
											onChange={(e) =>
												setFilters((f) => ({
													...f,
													[col.field]: e.target.value,
												}))
											}
											className="h-8"
											placeholder="Filter..."
										/>
									)}
								</div>
							))}
							{Object.values(filters).some(Boolean) && (
								<>
									<DropdownMenuSeparator />
									<DropdownMenuItem onClick={() => setFilters({})}>
										Clear filters
									</DropdownMenuItem>
								</>
							)}
						</DropdownMenuContent>
					</DropdownMenu>
				)}

				<div className="ml-auto flex items-center gap-2">
					<Button variant="outline" size="sm" onClick={load} disabled={loading}>
						{loading ? (
							<Loader2 className="size-3.5 animate-spin" />
						) : (
							<RefreshCw className="size-3.5" />
						)}
						Refresh
					</Button>
					<Button
						size="sm"
						onClick={() => {
							setInsertJson(emptyJson(manifestEntity));
							setInsertOpen(true);
						}}
					>
						<Plus className="size-3.5" /> Insert
					</Button>
				</div>
			</div>

			<Card>
				<CardContent className="p-0">
					{loading && rows.length === 0 ? (
						<div className="flex items-center justify-center py-16 text-muted-foreground">
							<Loader2 className="size-4 animate-spin" />
						</div>
					) : rows.length === 0 ? (
						<div className="py-16 text-center">
							<p className="text-sm font-medium">No rows in {entity} yet.</p>
							<p className="mt-1 text-xs text-muted-foreground">
								Insert one above to get started.
							</p>
						</div>
					) : (
						<Table>
							<TableHeader>
								<TableRow>
									<TableHead className="w-10">
										<input
											type="checkbox"
											className="size-4 cursor-pointer accent-primary"
											checked={allSelected}
											onChange={(e) => {
												if (e.target.checked) {
													setSelected(
														new Set(
															rows.map((r) => String(r.id ?? "")).filter(Boolean),
														),
													);
												} else {
													setSelected(new Set());
												}
											}}
										/>
									</TableHead>
									{visibleColumns.map((col) => {
										const sortActive = sort?.field === col.field;
										const SortIcon = sortActive
											? sort?.order === "asc"
												? ArrowUp
												: ArrowDown
											: ChevronsUpDown;
										return (
											<TableHead
												key={col.field}
												style={col.width ? { width: col.width } : undefined}
												className={
													col.align === "right"
														? "text-right"
														: col.align === "center"
															? "text-center"
															: undefined
												}
											>
												<button
													type="button"
													disabled={!col.sortable}
													onClick={() =>
														col.sortable && toggleSort(col.field, sort, setSort)
													}
													className={
														col.sortable
															? "inline-flex items-center gap-1 hover:text-foreground"
															: "inline-flex items-center gap-1"
													}
												>
													{labelFor(col)}
													{col.sortable && (
														<SortIcon className="size-3 opacity-70" />
													)}
												</button>
											</TableHead>
										);
									})}
									{listCfg?.rowActions?.length ? (
										<TableHead className="w-10" />
									) : (
										<TableHead className="w-10" />
									)}
								</TableRow>
							</TableHeader>
							<TableBody>
								{rows.map((row, i) => {
									const id = String(row.id ?? "");
									const checked = !!id && selected.has(id);
									return (
										<TableRow
											key={id || i}
											data-state={checked ? "selected" : undefined}
											className="cursor-pointer"
											onClick={() => setInspectRow(row)}
										>
											<TableCell
												className="w-10"
												onClick={(e) => e.stopPropagation()}
											>
												<input
													type="checkbox"
													className="size-4 cursor-pointer accent-primary"
													checked={checked}
													disabled={!id}
													onChange={(e) => {
														setSelected((prev) => {
															const next = new Set(prev);
															if (e.target.checked) next.add(id);
															else next.delete(id);
															return next;
														});
													}}
												/>
											</TableCell>
											{visibleColumns.map((col) => (
												<TableCell
													key={col.field}
													className={
														col.align === "right"
															? "text-right"
															: col.align === "center"
																? "text-center"
																: undefined
													}
												>
													<CellRenderer
														value={getField(row, col.field)}
														row={row}
														column={col}
													/>
												</TableCell>
											))}
											<TableCell
												className="text-right"
												onClick={(e) => e.stopPropagation()}
											>
												<RowActionsMenu
													row={row}
													actions={listCfg?.rowActions}
													onDelete={() => onDelete(row)}
												/>
											</TableCell>
										</TableRow>
									);
								})}
							</TableBody>
						</Table>
					)}
				</CardContent>
			</Card>

			<div className="flex flex-wrap items-center justify-between gap-3 px-1 text-sm">
				<div className="flex items-center gap-2">
					<span className="text-muted-foreground">Rows per page</span>
					<Select
						value={String(pageSize)}
						onValueChange={(v) => setPageSize(Number(v))}
					>
						<SelectTrigger className="h-8 w-[120px]">
							<SelectValue>{pageSize} / page</SelectValue>
						</SelectTrigger>
						<SelectContent>
							{pageSizes.map((s) => (
								<SelectItem key={s} value={String(s)}>
									{s} / page
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
				<div className="flex items-center gap-2 text-muted-foreground">
					Page {page} of {totalPages}
					<Button
						size="sm"
						variant="outline"
						disabled={page <= 1 || loading}
						onClick={() => setPage((p) => Math.max(1, p - 1))}
					>
						Previous
					</Button>
					<Button
						size="sm"
						variant="outline"
						disabled={page >= totalPages || loading}
						onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
					>
						Next
					</Button>
				</div>
			</div>

			<Dialog open={insertOpen} onOpenChange={setInsertOpen}>
				<DialogContent className="sm:max-w-[600px]">
					<DialogHeader>
						<DialogTitle>Insert into {entity}</DialogTitle>
						<DialogDescription>
							Paste a JSON object. Pre-filled with required fields from the
							schema.
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-3">
						<div className="rounded-md border bg-muted/30 p-3">
							<Label className="text-xs">Schema</Label>
							<div className="mt-1 flex flex-wrap gap-1.5">
								{manifestEntity.fields.map((f) => (
									<code
										key={f.name}
										className="rounded bg-background px-1.5 py-0.5 text-xs"
									>
										{f.name}
										{f.optional && "?"}: {f.type}
									</code>
								))}
							</div>
						</div>
						<div className="space-y-1.5">
							<Label htmlFor="insert-json">Body</Label>
							<Textarea
								id="insert-json"
								className="font-mono text-xs"
								rows={10}
								value={insertJson}
								onChange={(e) => setInsertJson(e.target.value)}
							/>
						</div>
					</div>
					<DialogFooter>
						<Button variant="ghost" onClick={() => setInsertOpen(false)}>
							Cancel
						</Button>
						<Button onClick={onInsert}>Insert</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			<RowEditor
				row={inspectRow}
				entity={manifestEntity}
				entityName={entity}
				onClose={() => setInspectRow(null)}
				onSaved={() => {
					setInspectRow(null);
					void load();
				}}
			/>

			<Dialog
				open={pendingBulk !== null}
				onOpenChange={(o) => !o && setPendingBulk(null)}
			>
				<DialogContent className="sm:max-w-[480px]">
					<DialogHeader>
						<DialogTitle>
							Delete {selected.size} {selected.size === 1 ? "row" : "rows"}?
						</DialogTitle>
						<DialogDescription>
							{pendingBulk?.confirm ??
								`This will permanently delete ${selected.size} ${entity} ${selected.size === 1 ? "row" : "rows"}. This action can't be undone.`}
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="ghost" onClick={() => setPendingBulk(null)}>
							Cancel
						</Button>
						<Button
							variant="destructive"
							onClick={() => {
								const action = pendingBulk;
								setPendingBulk(null);
								if (action) void runBulk(action);
							}}
						>
							Delete {selected.size}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function RowActionsMenu({
	actions,
	onDelete,
}: {
	row: Row;
	actions: RowAction[] | undefined;
	onDelete: () => void;
}) {
	if (!actions || actions.length === 0) {
		return (
			<Button variant="ghost" size="sm" onClick={onDelete}>
				<Trash2 className="size-3.5" />
			</Button>
		);
	}
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button variant="ghost" size="sm">
					<MoreHorizontal className="size-3.5" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end">
				{actions.map((a) => (
					<DropdownMenuItem
						key={a.id}
						onClick={() => {
							if (a.kind === "delete" || a.id === "delete") onDelete();
							else
								toast.info(
									`"${a.label}" — wire a custom handler in studio.entry.tsx`,
								);
						}}
					>
						{a.label}
					</DropdownMenuItem>
				))}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function toggleSort(
	field: string,
	current: { field: string; order: "asc" | "desc" } | null,
	setSort: (s: { field: string; order: "asc" | "desc" } | null) => void,
) {
	if (!current || current.field !== field) {
		setSort({ field, order: "asc" });
		return;
	}
	if (current.order === "asc") {
		setSort({ field, order: "desc" });
		return;
	}
	setSort(null);
}

function resolveColumns(
	entity: ManifestEntity | undefined,
	listCfg:
		| import("@/lib/studio-config").ResourceListConfig
		| undefined,
): ColumnConfig[] {
	if (listCfg?.columns && listCfg.columns.length > 0) {
		return [...listCfg.columns].sort(
			(a, b) => (a.order ?? 0) - (b.order ?? 0),
		);
	}
	if (!entity) return [];
	return [
		{ field: "id", label: "ID", renderer: { kind: "text", mono: true } },
		...entity.fields.map((f) => ({
			field: f.name,
			label: humanize(f.name),
			searchable: f.type === "string" || f.type === "richtext",
			sortable: !f.optional && (f.type === "string" || f.type === "datetime"),
			renderer: defaultRendererFor(f.type),
		})),
	];
}

function defaultRendererFor(type: string): ColumnConfig["renderer"] {
	if (type === "datetime") return { kind: "date", format: "relative" };
	if (type === "bool") return { kind: "boolean" };
	if (type === "int" || type === "float") return { kind: "number" };
	if (type === "richtext") return { kind: "text", truncate: 80 };
	return { kind: "text" };
}

function labelFor(col: ColumnConfig): string {
	return col.label ?? humanize(col.field);
}

function humanize(s: string): string {
	const out = s
		.replace(/[_-]/g, " ")
		.replace(/([a-z])([A-Z])/g, "$1 $2")
		.toLowerCase();
	return out.charAt(0).toUpperCase() + out.slice(1);
}

function getField(row: Row, field: string): unknown {
	if (!field.includes(".")) return row[field];
	let v: unknown = row;
	for (const seg of field.split(".")) {
		if (v && typeof v === "object" && seg in (v as Record<string, unknown>)) {
			v = (v as Record<string, unknown>)[seg];
		} else return undefined;
	}
	return v;
}

function clientFilter(
	rows: Row[],
	search: string,
	filters: Record<string, string>,
	columns: ColumnConfig[],
): Row[] {
	let out = rows;
	if (search.trim()) {
		const q = search.trim().toLowerCase();
		const fields = columns.filter((c) => c.searchable).map((c) => c.field);
		const fallback = fields.length > 0 ? fields : columns.map((c) => c.field);
		out = out.filter((r) =>
			fallback.some((f) => {
				const v = getField(r, f);
				return v != null && String(v).toLowerCase().includes(q);
			}),
		);
	}
	for (const [k, v] of Object.entries(filters)) {
		if (!v) continue;
		out = out.filter((r) => {
			const cell = getField(r, k);
			return cell != null && String(cell).toLowerCase() === v.toLowerCase();
		});
	}
	return out;
}

function clientSort(
	rows: Row[],
	sort: { field: string; order: "asc" | "desc" },
): Row[] {
	const out = [...rows];
	out.sort((a, b) => {
		const av = getField(a, sort.field);
		const bv = getField(b, sort.field);
		if (av == null && bv == null) return 0;
		if (av == null) return 1;
		if (bv == null) return -1;
		if (typeof av === "number" && typeof bv === "number")
			return sort.order === "asc" ? av - bv : bv - av;
		const as = String(av);
		const bs = String(bv);
		const cmp = as.localeCompare(bs);
		return sort.order === "asc" ? cmp : -cmp;
	});
	return out;
}

function emptyJson(entity: ManifestEntity): string {
	const obj: Record<string, string> = {};
	for (const f of entity.fields) {
		if (f.optional) continue;
		obj[f.name] = `<${f.type}>`;
	}
	return JSON.stringify(obj, null, 2);
}
