import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDown, ChevronRight, Loader2, Plus, RefreshCw, Save, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectSeparator,
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
import { toast } from "sonner";
import { ApiError, MANIFEST, type ManifestEntity, api } from "@/lib/pylon";

type Row = Record<string, unknown> & { id?: string };

function emptyJsonForEntity(entity: ManifestEntity): string {
	const obj: Record<string, string> = {};
	for (const f of entity.fields) {
		if (f.optional) continue;
		obj[f.name] = `<${f.type}>`;
	}
	return JSON.stringify(obj, null, 2);
}

/// Framework-internal auth tables. These don't appear in the manifest
/// (and shouldn't be exposed via /api/entities) but operators want to
/// inspect them from Studio for debugging "did the OAuth callback
/// actually create an account row" / "is there a stuck session." The
/// Rust side at /api/admin/auth/<key> handles read + redaction.
const AUTH_TABLES = [
	{ key: "accounts", label: "_pylon_accounts" },
	{ key: "sessions", label: "_pylon_sessions" },
	{ key: "magic_codes", label: "_pylon_magic_codes" },
	{ key: "oauth_state", label: "_pylon_oauth_state" },
] as const;
const AUTH_TABLE_PREFIX = "auth:";

/// Framework-internal operational tables. Same pattern as AUTH_TABLES
/// — read-only views over the job queue, workflow engine, scheduler,
/// and search-index registry. Surfaces in /api/admin/ops/<key>.
const OPS_TABLES = [
	{ key: "jobs", label: "Jobs" },
	{ key: "workflows", label: "Workflows" },
	{ key: "scheduler", label: "Scheduled tasks" },
	{ key: "search_indexes", label: "Search indexes" },
] as const;
const OPS_TABLE_PREFIX = "ops:";

function isAuthTable(selected: string): boolean {
	return selected.startsWith(AUTH_TABLE_PREFIX);
}

function authTableKey(selected: string): string {
	return selected.slice(AUTH_TABLE_PREFIX.length);
}

function isOpsTable(selected: string): boolean {
	return selected.startsWith(OPS_TABLE_PREFIX);
}

function opsTableKey(selected: string): string {
	return selected.slice(OPS_TABLE_PREFIX.length);
}

export function EntitiesPage() {
	const entities = MANIFEST.entities;
	const [selected, setSelected] = useState<string>(entities[0]?.name ?? "");
	const [rows, setRows] = useState<Row[]>([]);
	const [loading, setLoading] = useState(false);
	const [insertOpen, setInsertOpen] = useState(false);
	const [insertJson, setInsertJson] = useState("{}");
	const [inspectRow, setInspectRow] = useState<Row | null>(null);

	const isAuth = isAuthTable(selected);
	const isOps = isOpsTable(selected);
	const isFrameworkTable = isAuth || isOps;
	const entity = useMemo<ManifestEntity | undefined>(
		() =>
			isFrameworkTable
				? undefined
				: entities.find((e) => e.name === selected),
		[entities, selected, isFrameworkTable],
	);

	const load = useCallback(async () => {
		if (!selected) return;
		setLoading(true);
		try {
			let path: string;
			if (isAuth) {
				path = `/api/admin/auth/${authTableKey(selected)}`;
			} else if (isOps) {
				path = `/api/admin/ops/${opsTableKey(selected)}`;
			} else {
				path = `/api/entities/${selected}`;
			}
			const data = await api<Row[] | { data?: Row[] }>(path);
			setRows(Array.isArray(data) ? data : data?.data ?? []);
		} catch (err) {
			if (err instanceof ApiError) {
				toast.error(`${err.code}: ${err.message}`);
			} else {
				toast.error(err instanceof Error ? err.message : String(err));
			}
			setRows([]);
		} finally {
			setLoading(false);
		}
	}, [selected, isAuth, isOps]);

	useEffect(() => {
		void load();
	}, [load]);

	const onInsert = async () => {
		try {
			const data = JSON.parse(insertJson);
			await api(`/api/entities/${selected}`, {
				method: "POST",
				body: JSON.stringify(data),
			});
			toast.success(`Inserted into ${selected}`);
			setInsertOpen(false);
			setInsertJson("{}");
			void load();
		} catch (err) {
			if (err instanceof ApiError) {
				toast.error(`${err.code}: ${err.message}`);
			} else {
				toast.error(err instanceof Error ? err.message : String(err));
			}
		}
	};

	const onDelete = async (row: Row) => {
		if (!row.id) return;
		try {
			await api(`/api/entities/${selected}/${row.id}`, { method: "DELETE" });
			toast.success(`Deleted ${row.id}`);
			void load();
		} catch (err) {
			if (err instanceof ApiError) {
				toast.error(`${err.code}: ${err.message}`);
			} else {
				toast.error(err instanceof Error ? err.message : String(err));
			}
		}
	};

	const columns = useMemo(() => {
		if (rows.length === 0) {
			// Auth tables — column shape comes from the row schema, not
			// the manifest. We don't pre-render a header; empty-state
			// just says "no rows."
			return entity ? ["id", ...entity.fields.map((f) => f.name)] : [];
		}
		const seen = new Set<string>();
		const out: string[] = [];
		for (const r of rows) {
			for (const k of Object.keys(r)) {
				if (!seen.has(k)) {
					seen.add(k);
					out.push(k);
				}
			}
		}
		return out;
	}, [rows, entity]);

	const selectedLabel = isAuth
		? AUTH_TABLES.find((t) => t.key === authTableKey(selected))?.label ??
		  selected
		: isOps
			? OPS_TABLES.find((t) => t.key === opsTableKey(selected))?.label ??
			  selected
			: selected;

	if (entities.length === 0) {
		return (
			<Card>
				<CardHeader>
					<CardTitle>No entities defined</CardTitle>
					<CardDescription>
						Define entities in your Pylon manifest with{" "}
						<code className="rounded bg-muted px-1 py-0.5">entity()</code>{" "}
						to populate this page.
					</CardDescription>
				</CardHeader>
			</Card>
		);
	}

	return (
		<div className="space-y-4">
			<div className="flex flex-wrap items-center gap-2">
				<Select value={selected} onValueChange={setSelected}>
					<SelectTrigger className="w-full sm:w-[260px]">
						<SelectValue>{selectedLabel}</SelectValue>
					</SelectTrigger>
					<SelectContent>
						<SelectGroup>
							<SelectLabel className="text-xs text-muted-foreground">
								App entities
							</SelectLabel>
							{entities.map((e) => (
								<SelectItem key={e.name} value={e.name}>
									{e.name}
								</SelectItem>
							))}
						</SelectGroup>
						<SelectSeparator />
						<SelectGroup>
							<SelectLabel className="text-xs text-muted-foreground">
								Auth tables (framework, read-only)
							</SelectLabel>
							{AUTH_TABLES.map((t) => (
								<SelectItem
									key={t.key}
									value={`${AUTH_TABLE_PREFIX}${t.key}`}
								>
									<span className="font-mono text-xs">{t.label}</span>
								</SelectItem>
							))}
						</SelectGroup>
						<SelectSeparator />
						<SelectGroup>
							<SelectLabel className="text-xs text-muted-foreground">
								Operations (framework, read-only)
							</SelectLabel>
							{OPS_TABLES.map((t) => (
								<SelectItem
									key={t.key}
									value={`${OPS_TABLE_PREFIX}${t.key}`}
								>
									{t.label}
								</SelectItem>
							))}
						</SelectGroup>
					</SelectContent>
				</Select>
				<Button variant="outline" size="sm" onClick={load} disabled={loading}>
					{loading ? (
						<Loader2 className="size-3.5 animate-spin" />
					) : (
						<RefreshCw className="size-3.5" />
					)}
					Refresh
				</Button>
				<div className="ml-auto flex items-center gap-2">
					<span className="text-xs text-muted-foreground">
						{rows.length} row{rows.length === 1 ? "" : "s"}
					</span>
					{!isFrameworkTable && (
						<Button
							size="sm"
							onClick={() => {
								setInsertJson(entity ? emptyJsonForEntity(entity) : "{}");
								setInsertOpen(true);
							}}
						>
							<Plus className="size-3.5" /> Insert
						</Button>
					)}
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
							<p className="text-sm font-medium">
								No rows in {selected} yet.
							</p>
							<p className="mt-1 text-xs text-muted-foreground">
								Insert one above to get started.
							</p>
						</div>
					) : (
						<Table>
							<TableHeader>
								<TableRow>
									{columns.map((c) => (
										<TableHead key={c}>{c}</TableHead>
									))}
									<TableHead className="w-[1%]" />
								</TableRow>
							</TableHeader>
							<TableBody>
								{rows.map((row, i) => (
									<TableRow
										key={(row.id as string) ?? i}
										className="cursor-pointer"
										onClick={() => setInspectRow(row)}
									>
										{columns.map((c) => (
											<TableCell key={c} className="font-mono text-xs">
												{formatCell(row[c])}
											</TableCell>
										))}
										<TableCell className="text-right">
											{!isFrameworkTable && (
												<Button
													variant="ghost"
													size="sm"
													onClick={(e) => {
														e.stopPropagation();
														void onDelete(row);
													}}
												>
													<Trash2 className="size-3.5" />
												</Button>
											)}
										</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Table>
					)}
				</CardContent>
			</Card>

			<Dialog open={insertOpen} onOpenChange={setInsertOpen}>
				<DialogContent className="sm:max-w-[600px]">
					<DialogHeader>
						<DialogTitle>Insert into {selected}</DialogTitle>
						<DialogDescription>
							Paste a JSON object. Pre-filled with required fields from the
							schema.
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-3">
						{entity && (
							<div className="rounded-md border bg-muted/30 p-3">
								<Label className="text-xs">Schema</Label>
								<div className="mt-1 flex flex-wrap gap-1.5">
									{entity.fields.map((f) => (
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
						)}
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

			<RowEditorDialog
				row={inspectRow}
				entity={entity}
				readOnly={isFrameworkTable}
				onClose={() => setInspectRow(null)}
				onSaved={() => {
					setInspectRow(null);
					void load();
				}}
				selected={selected}
			/>
		</div>
	);
}

// ---------------------------------------------------------------------------
// Row editor — per-field inputs with type-aware widgets, plus a
// collapsible raw-JSON view for power users.
//
// PATCHes /api/entities/<entity>/<id> with only the fields the user
// changed. Read-only for framework tables (auth/ops) — those panels
// expose a JSON view but no Save button. Field types come from the
// manifest; unknown types fall back to a text input that round-trips
// the value through JSON.parse for non-string types when possible.
// ---------------------------------------------------------------------------
function RowEditorDialog({
	row,
	entity,
	readOnly,
	selected,
	onClose,
	onSaved,
}: {
	row: Row | null;
	entity: ManifestEntity | undefined;
	readOnly: boolean;
	selected: string;
	onClose: () => void;
	onSaved: () => void;
}) {
	const [draft, setDraft] = useState<Record<string, unknown>>({});
	const [showJson, setShowJson] = useState(false);
	const [saving, setSaving] = useState(false);

	// Reset the draft whenever a different row is opened. Keying on
	// row.id avoids stale draft state when the user clicks a second
	// row without closing the dialog first.
	useEffect(() => {
		if (row) setDraft(row);
	}, [row?.id]);

	if (!row) return null;

	const editableFields = entity
		? entity.fields.filter(
				(f) => f.name !== "id" && !f.name.startsWith("_"),
			)
		: [];

	const dirtyKeys = Object.keys(draft).filter((k) => {
		const before = (row as Record<string, unknown>)[k];
		const after = draft[k];
		return JSON.stringify(before) !== JSON.stringify(after);
	});

	const onChange = (name: string, value: unknown) => {
		setDraft((prev) => ({ ...prev, [name]: value }));
	};

	const onSave = async () => {
		if (!row.id || dirtyKeys.length === 0) return;
		setSaving(true);
		try {
			const patch: Record<string, unknown> = {};
			for (const k of dirtyKeys) patch[k] = draft[k];
			await api(`/api/entities/${selected}/${row.id}`, {
				method: "PATCH",
				body: JSON.stringify(patch),
			});
			toast.success(`Saved ${row.id}`);
			onSaved();
		} catch (err) {
			if (err instanceof ApiError) {
				toast.error(`${err.code}: ${err.message}`);
			} else {
				toast.error(err instanceof Error ? err.message : String(err));
			}
		} finally {
			setSaving(false);
		}
	};

	return (
		<Dialog open onOpenChange={(o) => !o && onClose()}>
			<DialogContent className="sm:max-w-[640px]">
				<DialogHeader>
					<DialogTitle>
						{readOnly ? "Inspect row" : "Edit row"}
					</DialogTitle>
					<DialogDescription className="font-mono text-xs">
						{(row.id as string) ?? "—"}
					</DialogDescription>
				</DialogHeader>

				{!readOnly && entity ? (
					<div className="max-h-[60vh] space-y-4 overflow-auto pr-1">
						{editableFields.map((f) => (
							<FieldEditor
								key={f.name}
								name={f.name}
								type={f.type}
								optional={Boolean(f.optional)}
								value={draft[f.name]}
								onChange={(v) => onChange(f.name, v)}
							/>
						))}

						<button
							type="button"
							onClick={() => setShowJson((s) => !s)}
							className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
						>
							{showJson ? (
								<ChevronDown className="size-3" />
							) : (
								<ChevronRight className="size-3" />
							)}
							{showJson ? "Hide" : "Show"} raw JSON
						</button>
						{showJson && (
							<pre className="max-h-[200px] overflow-auto rounded-md border bg-muted/30 p-3 text-xs">
								{JSON.stringify(draft, null, 2)}
							</pre>
						)}
					</div>
				) : (
					<pre className="max-h-[60vh] overflow-auto rounded-md border bg-muted/30 p-3 text-xs">
						{JSON.stringify(row, null, 2)}
					</pre>
				)}

				<DialogFooter className="gap-2 sm:gap-2">
					<Button variant="ghost" onClick={onClose}>
						{readOnly ? "Close" : "Cancel"}
					</Button>
					{!readOnly && (
						<Button
							onClick={onSave}
							disabled={saving || dirtyKeys.length === 0}
						>
							{saving ? (
								<Loader2 className="size-3.5 animate-spin" />
							) : (
								<Save className="size-3.5" />
							)}
							{dirtyKeys.length > 0
								? `Save ${dirtyKeys.length} change${dirtyKeys.length === 1 ? "" : "s"}`
								: "No changes"}
						</Button>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function FieldEditor({
	name,
	type,
	optional,
	value,
	onChange,
}: {
	name: string;
	type: string;
	optional: boolean;
	value: unknown;
	onChange: (v: unknown) => void;
}) {
	const isNullable = optional && value === null;

	const labelEl = (
		<div className="flex items-baseline justify-between">
			<Label htmlFor={`field-${name}`} className="text-xs">
				{name}
				{optional && <span className="text-muted-foreground"> (optional)</span>}
			</Label>
			<code className="text-[10px] text-muted-foreground">{type}</code>
		</div>
	);

	if (type === "bool") {
		return (
			<div className="space-y-1.5">
				{labelEl}
				<label className="inline-flex items-center gap-2 text-sm">
					<input
						id={`field-${name}`}
						type="checkbox"
						checked={value === true}
						onChange={(e) => onChange(e.target.checked)}
						className="size-4 rounded border-input"
					/>
					<span className="text-muted-foreground">
						{value === true ? "true" : value === false ? "false" : "(unset)"}
					</span>
				</label>
			</div>
		);
	}

	if (type === "int" || type === "float") {
		return (
			<div className="space-y-1.5">
				{labelEl}
				<Input
					id={`field-${name}`}
					type="number"
					step={type === "int" ? "1" : "any"}
					value={value === null || value === undefined ? "" : String(value)}
					onChange={(e) => {
						const raw = e.target.value;
						if (raw === "") {
							onChange(optional ? null : undefined);
							return;
						}
						const n = type === "int" ? parseInt(raw, 10) : parseFloat(raw);
						onChange(Number.isFinite(n) ? n : raw);
					}}
				/>
			</div>
		);
	}

	if (type === "datetime") {
		// HTML datetime-local needs the `YYYY-MM-DDTHH:mm` shape; our
		// manifest stores ISO 8601 with seconds + Z. Trim/expand on
		// each side of the input so round-tripping works.
		const isoToLocal = (v: unknown): string => {
			if (typeof v !== "string") return "";
			const m = v.match(/^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2})/);
			return m?.[1] ?? "";
		};
		return (
			<div className="space-y-1.5">
				{labelEl}
				<Input
					id={`field-${name}`}
					type="datetime-local"
					value={isoToLocal(value)}
					onChange={(e) => {
						const v = e.target.value;
						if (!v) {
							onChange(optional ? null : "");
							return;
						}
						// Append seconds + Z to match the canonical shape.
						onChange(`${v}:00Z`);
					}}
				/>
			</div>
		);
	}

	// Default: string-ish (string, richtext, id(X), or anything else).
	// `id(X)` rendered as a plain text input — could be upgraded to a
	// row-picker later, but text covers the common ops case (paste an
	// id from another row).
	return (
		<div className="space-y-1.5">
			{labelEl}
			<Input
				id={`field-${name}`}
				type="text"
				value={
					value === null || value === undefined
						? ""
						: typeof value === "string"
							? value
							: JSON.stringify(value)
				}
				onChange={(e) => {
					const v = e.target.value;
					if (v === "" && optional) {
						onChange(null);
						return;
					}
					onChange(v);
				}}
				placeholder={isNullable ? "(null)" : ""}
			/>
		</div>
	);
}

function formatCell(v: unknown): string {
	if (v === null || v === undefined) return "—";
	if (typeof v === "string") return v.length > 80 ? `${v.slice(0, 80)}…` : v;
	if (typeof v === "object") return JSON.stringify(v);
	return String(v);
}
