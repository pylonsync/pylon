import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	ArrowDown,
	ArrowUp,
	ChevronLeft,
	ChevronRight,
	ChevronsUpDown,
	ListFilter,
	MoreHorizontal,
	Plus,
	RefreshCw,
	Search,
	X,
} from "lucide-react";

import { ConfirmDialog } from "@/components/confirm-dialog";
import { Page, Toolbar } from "@/components/page";
import { AutoCell, CellRenderer } from "@/components/renderers";
import { RowEditor, type Row } from "@/components/RowEditor";
import { EmptyState, ErrorState, LoadingRows, Spinner } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
	Dialog,
	DialogBody,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
	Menu,
	MenuContent,
	MenuItem,
	MenuSeparator,
	MenuTrigger,
} from "@/components/ui/menu";
import {
	Popover,
	PopoverContent,
	PopoverTitle,
	PopoverTrigger,
} from "@/components/ui/popover";
import { Select } from "@/components/ui/select";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { toast } from "@/lib/toast";
import { Tooltip } from "@/components/ui/tooltip";
import { Icon } from "@/layout/icon";
import { setCount } from "@/lib/counts";
import { getExtensionAction } from "@/lib/extensions";
import { nextRequestNumber, useQuery } from "@/hooks/use-query";
import {
	clip,
	errorText,
	humanize,
	isNumericType,
	plural,
	shortId,
} from "@/lib/format";
import { MANIFEST, type ManifestEntity, api } from "@/lib/pylon";
import { forgetLabels, loadLabels, refTarget } from "@/lib/refs";
import {
	buildActionInput,
	copyToClipboard,
	getRowField,
	pickResultValue,
	resultToText,
} from "@/lib/row-action";
import type {
	BulkAction,
	ColumnConfig,
	ResourceListConfig,
	RowAction,
	StudioConfig,
} from "@/lib/studio-config";
import { cn } from "@/lib/utils";

const DEFAULT_PAGE_SIZES = [25, 50, 100, 250];
const DEFAULT_PAGE_SIZE = 50;

/// The list endpoint returns an envelope with the total when the request
/// has `page`. A server without pagination returns a bare array, and the
/// page then sorts, filters and pages the rows in the browser.
type ListResponse =
	| Row[]
	| { data: Row[]; total?: number | null; page?: number; per_page?: number };

type Sort = { field: string; order: "asc" | "desc" };

/** What the list shows: the page, and how the rows are narrowed and ordered. */
/** One filter on a field. `contains` matches a part of a text value. */
type FieldFilter = { op: "eq" | "contains"; value: string };

type View = {
	page: number;
	pageSize: number;
	search: string;
	sort: Sort | null;
	filters: Record<string, FieldFilter>;
};

/** A column of the table, with what the manifest says about its field. */
type Column = ColumnConfig & {
	/** The manifest type of the field, when the field is in the manifest. */
	type?: string;
	isId: boolean;
	/** The entity the field points to, when the field is a reference. */
	ref: string | null;
};

type Confirm = {
	title: string;
	description?: string;
	label: string;
	destructive?: boolean;
	run: () => unknown;
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

	const columns = useMemo(
		() => resolveColumns(manifestEntity, listCfg),
		[manifestEntity, listCfg],
	);
	const visibleColumns = useMemo(() => columns.filter((c) => !c.hidden), [columns]);
	// The page title is the name the sidebar shows for the entity.
	const pluralLabel = resourceCfg?.label ?? resourceCfg?.pluralLabel ?? entity;

	const pageSizes = listCfg?.pageSizes ?? DEFAULT_PAGE_SIZES;
	const [view, setView] = useState<View>(() => ({
		page: 1,
		pageSize:
			listCfg?.defaultPageSize ??
			(pageSizes.includes(DEFAULT_PAGE_SIZE) ? DEFAULT_PAGE_SIZE : pageSizes[0]),
		search: "",
		sort: listCfg?.defaultSort ?? null,
		filters: {},
	}));
	const { page, pageSize, search, sort, filters } = view;
	const [searchText, setSearchText] = useState("");
	const [selected, setSelected] = useState<Set<string>>(new Set());
	const [confirm, setConfirm] = useState<Confirm | null>(null);

	/** Change the sort, the filters, the search or the page size. The list goes to page 1. */
	const changeView = useCallback((patch: Partial<Omit<View, "page">>) => {
		setView((v) => ({ ...v, ...patch, page: 1 }));
		setSelected(new Set());
	}, []);
	const goToPage = (next: number) => {
		setView((v) => ({ ...v, page: next }));
		setSelected(new Set());
	};

	// The entity declares a search index, so the server answers `q`. Without
	// one the box filters the rows of the current page in the browser.
	const serverSearch = Boolean(manifestEntity?.search);

	// Typing waits a moment before the list changes.
	useEffect(() => {
		const t = window.setTimeout(() => {
			const next = searchText.trim();
			if (next === view.search) return;
			// A new search shows other rows. A row that is no longer shown
			// leaves the selection, so a bulk action cannot reach it.
			setSelected(new Set());
			setView((v) => ({ ...v, search: next, page: 1 }));
		}, 250);
		return () => window.clearTimeout(t);
	}, [searchText, view.search]);

	const params = new URLSearchParams();
	params.set("page", String(page));
	params.set("per_page", String(pageSize));
	if (sort) {
		params.set("sort", sort.field);
		params.set("order", sort.order);
	}
	if (serverSearch && search) params.set("q", search);
	for (const [k, f] of Object.entries(filters)) {
		params.set(`${f.op === "contains" ? "contains" : "filter"}[${k}]`, f.value);
	}
	const listUrl = `/api/entities/${encodeURIComponent(entity)}?${params.toString()}`;
	const unfiltered = !search && Object.keys(filters).length === 0;

	const {
		data,
		error,
		loading,
		reload: load,
		dataRequest: listRequest,
	} = useQuery(listUrl, async (): Promise<{ rows: Row[]; total: number | null }> => {
		const res = await api<ListResponse>(listUrl);
		if (Array.isArray(res)) {
			const filtered = clientFilter(res, filters);
			const sorted = sort ? clientSort(filtered, sort) : filtered;
			return {
				rows: sorted.slice((page - 1) * pageSize, page * pageSize),
				total: sorted.length,
			};
		}
		const total = typeof res.total === "number" ? res.total : null;
		if (unfiltered && total !== null) setCount(entity, total);
		return { rows: res.data ?? [], total };
	});
	const rows = data?.rows ?? null;
	const total = data?.total ?? null;

	// ---- Rows on screen -----------------------------------------------

	const shownRows = useMemo(() => {
		if (!rows) return [];
		if (serverSearch || !search) return rows;
		const q = search.toLowerCase();
		return rows.filter((r) =>
			visibleColumns.some((c) => {
				const v = getRowField(r, c.field);
				return (
					v != null &&
					(typeof v === "object" ? JSON.stringify(v) : String(v))
						.toLowerCase()
						.includes(q)
				);
			}),
		);
	}, [rows, search, serverSearch, visibleColumns]);

	// ---- Display values of referenced rows ------------------------------

	const [labels, setLabels] = useState<
		Record<string, Map<string, string | null>>
	>({});
	useEffect(() => {
		if (!rows || rows.length === 0) return;
		let live = true;
		const byTarget = new Map<string, string[]>();
		for (const col of columns) {
			if (!col.ref) continue;
			const ids = byTarget.get(col.ref) ?? [];
			for (const r of rows) {
				const v = getRowField(r, col.field);
				if (typeof v === "string" && v) ids.push(v);
			}
			byTarget.set(col.ref, ids);
		}
		for (const [target, ids] of byTarget) {
			if (ids.length === 0) continue;
			void loadLabels(MANIFEST, target, ids).then((map) => {
				if (live) setLabels((prev) => ({ ...prev, [target]: new Map(map) }));
			});
		}
		return () => {
			live = false;
		};
	}, [rows, columns]);
	const labelOf = useCallback(
		(target: string, id: string) => labels[target]?.get(id),
		[labels],
	);

	// ---- The row panel --------------------------------------------------
	//
	// The open row is `?row=<id>` in the URL, so a row has a link and a
	// reference cell in another table can open it.

	const [openId, setOpenId] = useState<string | null>(readRowParam);
	const [fetchedRow, setFetchedRow] = useState<Row | null>(null);
	const [creating, setCreating] = useState(false);
	const tableRef = useRef<HTMLDivElement>(null);
	const lastOpenId = useRef<string | null>(null);

	useEffect(() => {
		const onPop = () => setOpenId(readRowParam());
		window.addEventListener("popstate", onPop);
		return () => window.removeEventListener("popstate", onPop);
	}, []);

	const openRow = useCallback((id: string | null) => {
		const url = new URL(window.location.href);
		if (id) url.searchParams.set("row", id);
		else url.searchParams.delete("row");
		window.history.replaceState(null, "", url);
		if (id) {
			lastOpenId.current = id;
			setCreating(false);
		}
		setOpenId(id);
	}, []);

	const rowOnPage = openId
		? (rows?.find((r) => r.id === openId) ?? null)
		: null;
	// A row that is not on the current page is fetched by id.
	useEffect(() => {
		if (!openId || rowOnPage || rows === null) return;
		if (fetchedRow?.id === openId) return;
		let live = true;
		api<Row>(
			`/api/entities/${encodeURIComponent(entity)}/${encodeURIComponent(openId)}`,
		)
			.then((r) => {
				if (live && r) setFetchedRow(r);
			})
			.catch((err) => {
				if (!live) return;
				toast.error(`Studio could not open the row ${shortId(openId)}`, {
					description: errorText(err),
				});
				openRow(null);
			});
		return () => {
			live = false;
		};
	}, [openId, rowOnPage, rows, fetchedRow, entity, openRow]);
	// The row the server returned for the last save. The panel shows it at
	// once, before the list reloads, and keeps it if the reload fails. Only a
	// list from a request that started after the save replaces it.
	const [lastSave, setLastSave] = useState<{ row: Row; after: number } | null>(
		null,
	);
	const savedRow =
		lastSave && listRequest < lastSave.after && lastSave.row.id === openId
			? lastSave.row
			: null;
	const panelRow =
		savedRow ??
		rowOnPage ??
		(fetchedRow && fetchedRow.id === openId ? fetchedRow : null);

	const returnFocus = useCallback(() => {
		const id = lastOpenId.current;
		if (!id) return null;
		return (
			tableRef.current?.querySelector<HTMLElement>(
				`[data-row-id="${CSS.escape(id)}"]`,
			) ?? null
		);
	}, []);

	// ---- Delete, insert, bulk actions ------------------------------------

	const path = `/api/entities/${encodeURIComponent(entity)}`;

	const afterWrite = () => {
		forgetLabels(entity);
		void load();
	};

	const deleteRow = async (row: Row) => {
		if (!row.id) return;
		try {
			await api(`${path}/${encodeURIComponent(row.id)}`, { method: "DELETE" });
			toast.success("The row is deleted");
			if (openId === row.id) openRow(null);
			afterWrite();
		} catch (err) {
			toast.error("Studio could not delete the row", {
				description: errorText(err),
			});
		}
	};

	const askDeleteRow = (row: Row) =>
		setConfirm({
			title: "Delete the row?",
			description: `This deletes the ${entity} row ${shortId(String(row.id ?? ""))}. You cannot undo this.`,
			label: "Delete row",
			destructive: true,
			run: () => deleteRow(row),
		});

	const isDelete = (a: { kind?: string; id: string }) =>
		a.kind === "delete" || a.id === "delete";

	// One bulk action runs at a time. The bulk buttons are disabled while it
	// runs, so a second press cannot start the same writes again.
	const [bulkRunning, setBulkRunning] = useState(false);

	const runBulk = async (action: BulkAction) => {
		if (bulkRunning) return;
		setBulkRunning(true);
		try {
			await runBulkAction(action);
		} finally {
			setBulkRunning(false);
		}
	};

	const runBulkAction = async (action: BulkAction) => {
		const ids = Array.from(selected);
		if (ids.length === 0) return;
		if (isDelete(action)) {
			let ok = 0;
			let firstError: string | null = null;
			for (const id of ids) {
				try {
					await api(`${path}/${encodeURIComponent(id)}`, { method: "DELETE" });
					ok++;
				} catch (err) {
					firstError ??= errorText(err);
				}
			}
			if (ok === ids.length) {
				toast.success(`${plural(ok, "row")} deleted`);
			} else {
				toast.error(`${ok} of ${plural(ids.length, "row")} deleted`, {
					description: firstError ?? undefined,
				});
			}
			if (openId && ids.includes(openId)) openRow(null);
			setSelected(new Set());
			afterWrite();
			return;
		}
		if (action.kind === "export" || action.id === "export") {
			const subset = (rows ?? []).filter((r) => selected.has(String(r.id ?? "")));
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
		await runCustom(action, (rows ?? []).filter((r) => selected.has(String(r.id ?? ""))));
	};

	/** Run the handler that studio.entry.tsx registered for a custom action. */
	const runCustom = async (action: { id: string; label: string }, onRows: Row[]) => {
		const handler = getExtensionAction(action.id);
		if (!handler) return;
		try {
			await handler({
				entity,
				rows: onRows,
				api: api as <T>(p: string, o?: unknown) => Promise<T>,
				toast,
				refresh: async () => {
					forgetLabels(entity);
					await load();
				},
			});
		} catch (err) {
			toast.error(`${action.label} failed`, { description: errorText(err) });
		}
	};

	const onBulk = (action: BulkAction) => {
		const n = selected.size;
		if (n === 0) return;
		if (isDelete(action) || action.confirm) {
			setConfirm({
				title: isDelete(action) ? `Delete ${plural(n, "row")}?` : action.label,
				description:
					action.confirm ??
					`This deletes ${plural(n, `${entity} row`)}. You cannot undo this.`,
				label: isDelete(action) ? `Delete ${plural(n, "row")}` : action.label,
				destructive: isDelete(action),
				run: () => runBulk(action),
			});
			return;
		}
		void runBulk(action);
	};

	// ---- Row actions from studio.config.ts --------------------------------

	const [runningAction, setRunningAction] = useState<string | null>(null);
	const [resultDialog, setResultDialog] = useState<{
		title: string;
		text: string;
	} | null>(null);

	const presentResult = async (action: RowAction, value: unknown) => {
		const mode = action.result ?? "toast";
		if (mode === "none") {
			toast.success(action.label);
			return;
		}
		const text = resultToText(pickResultValue(value, action.resultField));
		if (mode === "dialog") {
			setResultDialog({ title: action.label, text: text || "(no output)" });
			return;
		}
		if (mode === "copy") {
			if (!text) {
				toast.info(`${action.label}: there is nothing to copy`);
				return;
			}
			if (await copyToClipboard(text)) {
				toast.success("Copied to the clipboard", { description: clip(text, 120) });
			} else {
				// The clipboard is not available over plain http on a LAN
				// address. The dialog shows the value to select and copy.
				setResultDialog({ title: action.label, text });
			}
			return;
		}
		toast.success(action.label, {
			description: text ? clip(text, 200) : undefined,
		});
	};

	const runRowAction = async (action: RowAction, row: Row) => {
		if (isDelete(action)) {
			await deleteRow(row);
			return;
		}
		if (action.kind === "view" || action.kind === "edit") {
			if (row.id) openRow(row.id);
			return;
		}
		if (action.kind !== "action") {
			setRunningAction(runKey(action, row));
			try {
				await runCustom(action, [row]);
			} finally {
				setRunningAction(null);
			}
			return;
		}
		// `action` names the function. With no `action`, the `id` is the name.
		const fnName = action.action ?? action.id;
		setRunningAction(runKey(action, row));
		try {
			const value = await api<unknown>(`/api/fn/${encodeURIComponent(fnName)}`, {
				method: "POST",
				body: JSON.stringify(buildActionInput(action, row)),
			});
			await presentResult(action, value);
			if (action.refresh !== false) afterWrite();
		} catch (err) {
			toast.error(`${action.label} failed`, { description: errorText(err) });
		} finally {
			setRunningAction(null);
		}
	};

	const onRowAction = (action: RowAction, row: Row) => {
		if (action.confirm || isDelete(action)) {
			const destructive = action.variant === "destructive" || isDelete(action);
			setConfirm({
				title: action.confirm ? action.label : "Delete the row?",
				description:
					action.confirm ??
					`This deletes the ${entity} row ${shortId(String(row.id ?? ""))}. You cannot undo this.`,
				label: action.label,
				destructive,
				run: () => runRowAction(action, row),
			});
			return;
		}
		void runRowAction(action, row);
	};

	if (!manifestEntity) {
		return (
			<Page title={entity}>
				<EmptyState
					title={`The entity "${entity}" is not in the manifest`}
					description="Select an entity from the sidebar."
				/>
			</Page>
		);
	}

	const rowActions = listCfg?.rowActions ?? [];
	const buttonActions = rowActions.filter((a) => a.display === "button");
	const menuActions = rowActions.filter((a) => a.display !== "button");

	const totalPages = total !== null ? Math.max(1, Math.ceil(total / pageSize)) : 1;
	const selectable = shownRows.filter((r) => r.id).map((r) => String(r.id));
	const allSelected =
		selectable.length > 0 && selectable.every((id) => selected.has(id));
	const someSelected = selectable.some((id) => selected.has(id));

	const filterableColumns = visibleColumns.filter((c) => c.filterable);
	const showFilters =
		(listCfg?.filterable ?? filterableColumns.length > 0) &&
		filterableColumns.length > 0;
	const showSearch =
		listCfg?.searchable ?? (serverSearch || visibleColumns.some((c) => c.searchable));
	const activeFilters = Object.entries(filters);
	const narrowed = activeFilters.length > 0 || Boolean(search);

	const first = total === 0 ? 0 : (page - 1) * pageSize + 1;
	const last = (page - 1) * pageSize + (rows?.length ?? 0);

	return (
		<Page
			title={pluralLabel}
			meta={total !== null ? plural(total, "row") : undefined}
			actions={
				<>
					<Tooltip label="Load the rows again">
						<Button
							variant="ghost"
							size="icon"
							aria-label="Refresh"
							disabled={loading}
							onClick={load}
						>
							{loading ? <Spinner /> : <RefreshCw />}
						</Button>
					</Tooltip>
					<Button
						onClick={() => {
							openRow(null);
							setCreating(true);
						}}
					>
						<Plus />
						Insert row
					</Button>
				</>
			}
		>
			{(showSearch || showFilters || selected.size > 0) && (
				<Toolbar>
					{selected.size > 0 ? (
						<>
							<span className="px-1 text-[13px] font-medium tabular-nums">
								{selected.size} selected
							</span>
							{(listCfg?.bulkActions ?? []).map((a) => {
								const missing = missingHandler(a);
								const button = (
									<Button
										key={a.id}
										size="sm"
										variant={isDelete(a) ? "destructive" : "outline"}
										disabled={missing !== null || bulkRunning}
										focusableWhenDisabled
										onClick={() => onBulk(a)}
									>
										{a.label}
									</Button>
								);
								return missing ? (
									<Tooltip key={a.id} label={missing}>
										{button}
									</Tooltip>
								) : (
									button
								);
							})}
							<Button
								size="sm"
								variant="outline"
								disabled={bulkRunning}
								onClick={() =>
									onBulk({ id: "export", label: "Export JSON", kind: "export" })
								}
							>
								Export JSON
							</Button>
							{!(listCfg?.bulkActions ?? []).some(isDelete) && (
								<Button
									size="sm"
									variant="outline"
									className="text-destructive"
									disabled={bulkRunning}
									onClick={() =>
										onBulk({ id: "delete", label: "Delete", kind: "delete" })
									}
								>
									Delete
								</Button>
							)}
							<Button
								size="sm"
								variant="ghost"
								className="ml-auto"
								onClick={() => setSelected(new Set())}
							>
								Clear selection
							</Button>
						</>
					) : (
						<>
							{showSearch && (
								<div className="relative min-w-0 flex-1 sm:w-64 sm:flex-none">
									<Search className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
									<Input
										type="search"
										name="search"
										aria-label={serverSearch ? "Search" : "Find in this page"}
										placeholder={serverSearch ? "Search" : "Find in this page"}
										value={searchText}
										onChange={(e) => setSearchText(e.target.value)}
										className="pl-7"
									/>
								</div>
							)}
							{showFilters && (
								<FilterAdd
									columns={filterableColumns}
									onAdd={(field, filter) =>
										changeView({ filters: { ...filters, [field]: filter } })
									}
								/>
							)}
							{activeFilters.map(([field, { op, value }]) => (
								<span
									key={field}
									className="inline-flex h-6 items-center gap-1 rounded-md border bg-muted pr-0.5 pl-2 text-xs"
								>
									<span className="text-muted-foreground">
										{labelFor(columns.find((c) => c.field === field), field)}
										{op === "contains" ? " contains" : " ="}
									</span>
									<span className="max-w-40 truncate font-medium">{value}</span>
									<button
										type="button"
										aria-label={`Remove the ${field} filter`}
										className="flex size-5 cursor-default items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
										onClick={() => {
											const next = { ...filters };
											delete next[field];
											changeView({ filters: next });
										}}
									>
										<X className="size-3" />
									</button>
								</span>
							))}
							{activeFilters.length > 1 && (
								<Button size="sm" variant="ghost" onClick={() => changeView({ filters: {} })}>
									Clear filters
								</Button>
							)}
						</>
					)}
				</Toolbar>
			)}

			<div ref={tableRef} className="min-h-0 flex-1 overflow-auto">
				{error && (rows === null || !loading) ? (
					<ErrorState
						title={`Studio could not load the ${entity} rows`}
						message={error}
						onRetry={load}
					/>
				) : rows === null ? (
					<LoadingRows rows={12} />
				) : shownRows.length === 0 ? (
					narrowed ? (
						<EmptyState
							title="No rows match"
							description={
								serverSearch
									? "Change the search text or remove a filter."
									: "The box finds text in the rows of this page only. Add a filter to search every row."
							}
							action={
								<Button
									variant="outline"
									onClick={() => {
										setSearchText("");
										changeView({ search: "", filters: {} });
									}}
								>
									Clear search and filters
								</Button>
							}
						/>
					) : (
						<EmptyState
							title={`${entity} has no rows`}
							action={
								<Button variant="outline" onClick={() => setCreating(true)}>
									<Plus />
									Insert row
								</Button>
							}
						/>
					)
				) : (
					<Table aria-label={pluralLabel} aria-busy={loading}>
						<TableHeader>
							<tr>
								<TableHead className="z-30 px-0 sm:sticky sm:left-0">
									<div className="flex w-9 justify-center">
										<Checkbox
											aria-label="Select all rows on this page"
											checked={
												allSelected ? true : someSelected ? "indeterminate" : false
											}
											onCheckedChange={(checked) =>
												setSelected(checked === true ? new Set(selectable) : new Set())
											}
										/>
									</div>
								</TableHead>
								{visibleColumns.map((col, i) => {
									const active = sort?.field === col.field;
									const SortIcon = active
										? sort?.order === "asc"
											? ArrowUp
											: ArrowDown
										: ChevronsUpDown;
									const align = alignOf(col);
									return (
										<TableHead
											key={col.field}
											style={col.width ? { width: col.width } : undefined}
											aria-sort={
												active
													? sort?.order === "asc"
														? "ascending"
														: "descending"
													: undefined
											}
											className={cn(
												align === "right" && "text-right",
												align === "center" && "text-center",
												i === 0 &&
													"z-30 border-r sm:sticky sm:left-9",
											)}
										>
											{col.sortable ? (
												<button
													type="button"
													className={cn(
														"group/sort -mx-1 inline-flex h-6 cursor-default items-center gap-1 rounded px-1 hover:bg-accent hover:text-foreground",
														align === "right" && "flex-row-reverse",
														active && "text-foreground",
													)}
													onClick={() => changeView({ sort: nextSort(col.field, sort) })}
												>
													{labelFor(col)}
													<SortIcon
														className={cn(
															"size-3",
															!active && "opacity-0 group-hover/sort:opacity-60",
														)}
													/>
												</button>
											) : (
												labelFor(col)
											)}
										</TableHead>
									);
								})}
								<TableHead className="w-px px-1 sm:sticky sm:right-0">
									<span className="sr-only">Actions</span>
								</TableHead>
							</tr>
						</TableHeader>
						<TableBody>
							{shownRows.map((row, index) => {
								const id = String(row.id ?? "");
								const checked = !!id && selected.has(id);
								return (
									<TableRow
										key={id || index}
										tabIndex={0}
										data-row-id={id || undefined}
										data-selected={checked}
										data-current={!!id && id === openId}
										className="cursor-default"
										onClick={() => id && openRow(id)}
										onKeyDown={(e) => {
											if (e.target !== e.currentTarget) return;
											if (e.key === "Enter" || e.key === " ") {
												e.preventDefault();
												if (id) openRow(id);
											} else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
												e.preventDefault();
												const sibling =
													e.key === "ArrowDown"
														? e.currentTarget.nextElementSibling
														: e.currentTarget.previousElementSibling;
												if (sibling instanceof HTMLElement) sibling.focus();
											}
										}}
									>
										<TableCell
											className="z-10 px-0 sm:sticky sm:left-0"
											onClick={(e) => e.stopPropagation()}
										>
											<div className="flex w-9 justify-center">
												<Checkbox
													aria-label="Select the row"
													checked={checked}
													disabled={!id}
													onCheckedChange={(next) =>
														setSelected((prev) => {
															const out = new Set(prev);
															if (next === true) out.add(id);
															else out.delete(id);
															return out;
														})
													}
												/>
											</div>
										</TableCell>
										{visibleColumns.map((col, i) => {
											const value = getRowField(row, col.field);
											const align = alignOf(col);
											return (
												<TableCell
													key={col.field}
													className={cn(
														align === "right" && "text-right",
														align === "center" && "text-center",
														i === 0 && "z-10 border-r sm:sticky sm:left-9",
													)}
												>
													{col.renderer ? (
														<CellRenderer value={value} row={row} column={col} />
													) : (
														<AutoCell
															value={value}
															type={col.type}
															isId={col.isId}
															refEntity={col.ref}
															refLabel={
																col.ref && typeof value === "string"
																	? labelOf(col.ref, value)
																	: undefined
															}
														/>
													)}
												</TableCell>
											);
										})}
										<TableCell
											className="z-10 w-px px-1 sm:sticky sm:right-0"
											onClick={(e) => e.stopPropagation()}
										>
											<div className="flex items-center justify-end gap-1">
												{buttonActions.map((a) => (
													<RowActionButton
														key={a.id}
														action={a}
														running={runningAction === runKey(a, row)}
														disabled={runningAction !== null}
														onRun={() => onRowAction(a, row)}
													/>
												))}
												{rowActions.length === 0 ? (
													<DefaultRowMenu
														row={row}
														onOpen={() => id && openRow(id)}
														onDelete={() => askDeleteRow(row)}
													/>
												) : (
													menuActions.length > 0 && (
														<ConfiguredRowMenu
															actions={menuActions}
															onRun={(a) => onRowAction(a, row)}
														/>
													)
												)}
											</div>
										</TableCell>
									</TableRow>
								);
							})}
						</TableBody>
					</Table>
				)}
			</div>

			<footer className="flex min-h-9 shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-t px-3 py-1 text-xs text-muted-foreground tabular-nums">
				<span>
					{rows === null
						? "Loading"
						: total === null
							? plural(rows.length, "row")
							: total === 0
								? "0 rows"
								: `${first.toLocaleString()}–${last.toLocaleString()} of ${total.toLocaleString()}`}
				</span>
				<div className="ml-auto flex items-center gap-1.5">
					<Select
						aria-label="Rows per page"
						className="h-6 text-xs"
						value={String(pageSize)}
						onValueChange={(v) => changeView({ pageSize: Number(v) })}
						options={pageSizes.map((s) => ({
							value: String(s),
							label: `${s} per page`,
						}))}
					/>
					<span className="px-1">
						Page {page} of {totalPages}
					</span>
					<Button
						variant="outline"
						size="icon-sm"
						aria-label="Previous page"
						disabled={page <= 1 || loading}
						onClick={() => goToPage(Math.max(1, page - 1))}
					>
						<ChevronLeft />
					</Button>
					<Button
						variant="outline"
						size="icon-sm"
						aria-label="Next page"
						disabled={page >= totalPages || loading}
						onClick={() => goToPage(Math.min(totalPages, page + 1))}
					>
						<ChevronRight />
					</Button>
				</div>
			</footer>

			<RowEditor
				open={creating || (openId !== null && panelRow !== null)}
				mode={creating ? "create" : "edit"}
				row={creating ? null : panelRow}
				entity={manifestEntity}
				entityName={entity}
				refLabels={labelOf}
				returnFocus={returnFocus}
				onClose={() => {
					setCreating(false);
					openRow(null);
				}}
				onDelete={askDeleteRow}
				onSaved={(saved) => {
					if (creating) {
						setCreating(false);
						if (saved?.id) lastOpenId.current = saved.id;
					} else if (saved && saved.id === openId) {
						setFetchedRow(saved);
						setLastSave({ row: saved, after: nextRequestNumber() });
					}
					afterWrite();
				}}
			/>

			<ConfirmDialog
				open={confirm !== null}
				title={confirm?.title ?? ""}
				description={confirm?.description}
				confirmLabel={confirm?.label ?? ""}
				destructive={confirm?.destructive}
				onCancel={() => setConfirm(null)}
				onConfirm={async () => {
					const pending = confirm;
					if (pending) await pending.run();
					setConfirm(null);
				}}
			/>

			<Dialog
				open={resultDialog !== null}
				onOpenChange={(o) => !o && setResultDialog(null)}
			>
				<DialogContent className="max-w-xl">
					<DialogHeader>
						<DialogTitle>{resultDialog?.title}</DialogTitle>
					</DialogHeader>
					<DialogBody>
						<Textarea
							readOnly
							rows={8}
							aria-label="Result"
							className="font-mono text-xs"
							value={resultDialog?.text ?? ""}
							onFocus={(e) => e.currentTarget.select()}
						/>
					</DialogBody>
					<DialogFooter>
						<Button
							variant="outline"
							onClick={async () => {
								if (await copyToClipboard(resultDialog?.text ?? "")) {
									toast.success("Copied to the clipboard");
								} else {
									toast.error("The clipboard is not available", {
										description: "Select the text and copy it.",
									});
								}
							}}
						>
							Copy
						</Button>
						<Button onClick={() => setResultDialog(null)}>Close</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</Page>
	);
}

// ---------------------------------------------------------------------------
// Parts
// ---------------------------------------------------------------------------

/** The Filter button: pick a field, give a value, add the filter. */
function FilterAdd({
	columns,
	onAdd,
}: {
	columns: Column[];
	onAdd: (field: string, filter: FieldFilter) => void;
}) {
	const [open, setOpen] = useState(false);
	const [field, setField] = useState(columns[0]?.field ?? "");
	const [op, setOp] = useState<FieldFilter["op"]>("contains");
	const [value, setValue] = useState("");
	const column = columns.find((c) => c.field === field) ?? columns[0];
	// A part of a value can match only in a text field.
	const text = column !== undefined && isTextColumn(column);
	const effectiveOp = text ? op : "eq";
	const options =
		typeof column?.filterable === "object" && column.filterable.options
			? column.filterable.options.map((o) => ({
					value: String(o.value),
					label: o.label,
				}))
			: column?.type === "bool"
				? [
						{ value: "true", label: "true" },
						{ value: "false", label: "false" },
					]
				: null;

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger
				render={
					<Button variant="outline">
						<ListFilter />
						Filter
					</Button>
				}
			/>
			<PopoverContent align="start" className="w-72">
				<PopoverTitle className="sr-only">Add a filter</PopoverTitle>
				<form
					className="space-y-2.5"
					onSubmit={(e) => {
						e.preventDefault();
						if (!column || !value) return;
						onAdd(column.field, { op: effectiveOp, value });
						setValue("");
						setOpen(false);
					}}
				>
					<div className="space-y-1">
						<Label htmlFor="filter-field">Field</Label>
						<Select
							id="filter-field"
							className="w-full"
							value={column?.field ?? null}
							onValueChange={(next) => {
								setField(next);
								setValue("");
							}}
							options={columns.map((c) => ({
								value: c.field,
								label: labelFor(c),
							}))}
						/>
					</div>
					<div className="space-y-1">
						<Label htmlFor="filter-op">Condition</Label>
						<Select
							id="filter-op"
							className="w-full"
							value={effectiveOp}
							disabled={!text}
							onValueChange={(next) => setOp(next as FieldFilter["op"])}
							options={[
								{ value: "contains", label: "Contains", disabled: !text },
								{ value: "eq", label: "Is equal to" },
							]}
						/>
					</div>
					<div className="space-y-1">
						<Label htmlFor="filter-value">Value</Label>
						{options ? (
							<Select
								id="filter-value"
								className="w-full"
								placeholder="Select a value"
								value={value || null}
								onValueChange={setValue}
								options={options}
							/>
						) : (
							<Input
								id="filter-value"
								name="value"
								value={value}
								autoFocus
								onChange={(e) => setValue(e.target.value)}
							/>
						)}
					</div>
					<div className="flex justify-end">
						<Button type="submit" disabled={!value}>
							Add filter
						</Button>
					</div>
				</form>
			</PopoverContent>
		</Popover>
	);
}

/**
 * Why a custom action cannot run, or `null` when it can. A `kind: "custom"`
 * action runs the handler that studio.entry.tsx registers under its id.
 */
function missingHandler(action: { id: string; kind?: string }): string | null {
	const builtIn = ["delete", "export", "edit", "view", "action"];
	if (builtIn.includes(action.kind ?? "") || builtIn.includes(action.id)) return null;
	return getExtensionAction(action.id)
		? null
		: `No handler "${action.id}" is registered in studio.entry.tsx.`;
}

/** A row action that shows as a button in the row. */
function RowActionButton({
	action,
	running,
	disabled,
	onRun,
}: {
	action: RowAction;
	running: boolean;
	disabled: boolean;
	onRun: () => void;
}) {
	const missing = missingHandler(action);
	const button = (
		<Button
			size="sm"
			variant={action.variant === "default" ? "default" : (action.variant ?? "outline")}
			disabled={disabled || missing !== null}
			focusableWhenDisabled={missing !== null}
			onClick={onRun}
		>
			{running ? <Spinner /> : action.icon ? <Icon name={action.icon} /> : null}
			{action.label}
		</Button>
	);
	return missing ? <Tooltip label={missing}>{button}</Tooltip> : button;
}

function RowMenuTrigger() {
	return (
		<MenuTrigger
			aria-label="Row actions"
			className="flex size-6 cursor-default items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground data-open:bg-accent data-open:text-foreground"
		>
			<MoreHorizontal className="size-3.5" />
		</MenuTrigger>
	);
}

/** The row menu of an entity with no `rowActions` in studio.config.ts. */
function DefaultRowMenu({
	row,
	onOpen,
	onDelete,
}: {
	row: Row;
	onOpen: () => void;
	onDelete: () => void;
}) {
	return (
		<Menu>
			<RowMenuTrigger />
			<MenuContent align="end">
				<MenuItem onSelect={onOpen}>Open</MenuItem>
				<MenuItem
					disabled={!row.id}
					onSelect={async () => {
						if (await copyToClipboard(String(row.id ?? ""))) {
							toast.success("Copied the id");
						} else {
							toast.error("The clipboard is not available", {
								description: String(row.id ?? ""),
							});
						}
					}}
				>
					Copy the id
				</MenuItem>
				<MenuItem
					onSelect={async () => {
						if (await copyToClipboard(JSON.stringify(row, null, 2))) {
							toast.success("Copied the row as JSON");
						} else {
							toast.error("The clipboard is not available");
						}
					}}
				>
					Copy as JSON
				</MenuItem>
				<MenuSeparator />
				<MenuItem destructive disabled={!row.id} onSelect={onDelete}>
					Delete
				</MenuItem>
			</MenuContent>
		</Menu>
	);
}

function ConfiguredRowMenu({
	actions,
	onRun,
}: {
	actions: RowAction[];
	onRun: (action: RowAction) => void;
}) {
	return (
		<Menu>
			<RowMenuTrigger />
			<MenuContent align="end">
				{actions.map((a) => (
					<MenuItem
						key={a.id}
						destructive={a.kind === "delete" || a.id === "delete"}
						disabled={missingHandler(a) !== null}
						title={missingHandler(a) ?? undefined}
						onSelect={() => onRun(a)}
					>
						{a.icon && <Icon name={a.icon} />}
						{a.label}
					</MenuItem>
				))}
			</MenuContent>
		</Menu>
	);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function readRowParam(): string | null {
	return new URLSearchParams(window.location.search).get("row");
}

/** A key for one action on one row. It selects the button that shows a spinner. */
function runKey(action: RowAction, row: Row): string {
	// NUL separates the parts: an action id or a row id can hold any
	// printable character.
	return `${action.id}\u0000${String(row.id ?? "")}`;
}

function nextSort(field: string, current: Sort | null): Sort | null {
	if (!current || current.field !== field) return { field, order: "asc" };
	if (current.order === "asc") return { field, order: "desc" };
	return null;
}

function alignOf(col: Column): "left" | "center" | "right" {
	if (col.align) return col.align;
	if (col.renderer?.kind === "number") return "right";
	return !col.renderer && !col.ref && isNumericType(col.type) ? "right" : "left";
}

function labelFor(col: ColumnConfig | undefined, fallback = ""): string {
	if (!col) return humanize(fallback);
	return col.label ?? humanize(col.field);
}

const FILTERABLE_TYPES = new Set(["string", "richtext", "bool", "int", "float"]);

/**
 * The columns of the table. Columns from studio.config.ts win. With no
 * config, the table has the id and one column for each manifest field.
 */
function resolveColumns(
	entity: ManifestEntity | undefined,
	listCfg: ResourceListConfig | undefined,
): Column[] {
	const typeOf = (field: string) =>
		entity?.fields.find((f) => f.name === field)?.type;

	if (listCfg?.columns && listCfg.columns.length > 0) {
		return [...listCfg.columns]
			.sort((a, b) => (a.order ?? 0) - (b.order ?? 0))
			.map((c) => ({
				...c,
				type: typeOf(c.field),
				isId: c.field === "id",
				ref: refTarget(MANIFEST, entity, c.field),
			}));
	}
	if (!entity) return [];
	return [
		{ field: "id", label: "ID", isId: true, ref: null },
		...entity.fields
			.filter((f) => !f.name.startsWith("_"))
			.map((f) => {
				const ref = refTarget(MANIFEST, entity, f.name);
				return {
					field: f.name,
					label: humanize(ref ? f.name.replace(/_?[iI]d$/, "") || f.name : f.name),
					type: f.type,
					isId: false,
					ref,
					searchable: f.type === "string" || f.type === "richtext",
					sortable: f.type !== "json",
					filterable: FILTERABLE_TYPES.has(f.type) || ref !== null,
				};
			}),
	];
}

function clientFilter(rows: Row[], filters: Record<string, FieldFilter>): Row[] {
	let out = rows;
	for (const [k, { op, value }] of Object.entries(filters)) {
		const want = value.toLowerCase();
		out = out.filter((r) => {
			const cell = getRowField(r, k);
			if (cell == null) return false;
			const have = String(cell).toLowerCase();
			return op === "contains" ? have.includes(want) : have === want;
		});
	}
	return out;
}

/** Whether a column holds text, so a filter can match a part of its value. */
function isTextColumn(col: Column): boolean {
	if (col.ref || col.isId) return false;
	if (typeof col.filterable === "object" && col.filterable.options) return false;
	return col.type === undefined || col.type === "string" || col.type === "richtext";
}

function clientSort(rows: Row[], sort: Sort): Row[] {
	const out = [...rows];
	out.sort((a, b) => {
		const av = getRowField(a, sort.field);
		const bv = getRowField(b, sort.field);
		if (av == null && bv == null) return 0;
		if (av == null) return 1;
		if (bv == null) return -1;
		if (typeof av === "number" && typeof bv === "number") {
			return sort.order === "asc" ? av - bv : bv - av;
		}
		const cmp = String(av).localeCompare(String(bv));
		return sort.order === "asc" ? cmp : -cmp;
	});
	return out;
}
