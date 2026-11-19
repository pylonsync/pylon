import { useMemo, useRef, useState } from "react";
import { MoreHorizontal, Plus, RefreshCw } from "lucide-react";

import { useAuth } from "@/auth/auth";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { Page, Toolbar } from "@/components/page";
import { AutoCell } from "@/components/renderers";
import { RowEditor, type Row } from "@/components/RowEditor";
import {
	EmptyState,
	ErrorState,
	LoadingRows,
	Spinner,
} from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Menu,
	MenuContent,
	MenuItem,
	MenuSeparator,
	MenuTrigger,
} from "@/components/ui/menu";
import { Select } from "@/components/ui/select";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { toast } from "@/lib/toast";
import { Tooltip } from "@/components/ui/tooltip";
import { useQuery } from "@/hooks/use-query";
import { errorText, plural, shortId } from "@/lib/format";
import { MANIFEST, type ManifestEntity, api } from "@/lib/pylon";
import { refTarget } from "@/lib/refs";
import { LockedPage } from "./Locked";

/// Auth tables of the framework. They are not in the manifest and
/// `/api/entities` does not serve them. `/api/admin/auth/<key>` returns
/// their rows with the secrets removed.
const AUTH_TABLES = [
	{ key: "accounts", label: "_pylon_accounts" },
	{ key: "sessions", label: "_pylon_sessions" },
	{ key: "magic_codes", label: "_pylon_magic_codes" },
	{ key: "oauth_state", label: "_pylon_oauth_state" },
] as const;
const AUTH_PREFIX = "auth:";

/// Operations tables of the framework: the job queue, the workflow engine,
/// the scheduler and the search indexes. `/api/admin/ops/<key>` serves them.
const OPS_TABLES = [
	{ key: "jobs", label: "Jobs" },
	{ key: "workflows", label: "Workflows" },
	{ key: "scheduler", label: "Scheduled tasks" },
	{ key: "search_indexes", label: "Search indexes" },
] as const;
const OPS_PREFIX = "ops:";

/**
 * Whether a column of a framework table holds a time. The framework tables
 * have no manifest types. Their time columns end in `_at` or `_run` and hold
 * a Unix time.
 */
function isTimeColumn(name: string, value: unknown): boolean {
	if (!/(_at|_run)$/.test(name)) return false;
	return typeof value === "number"
		? value > 1_000_000_000
		: typeof value === "string" && /^\d{9,13}Z?$/.test(value);
}

function tablePath(selected: string): string {
	if (selected.startsWith(AUTH_PREFIX)) {
		return `/api/admin/auth/${selected.slice(AUTH_PREFIX.length)}`;
	}
	if (selected.startsWith(OPS_PREFIX)) {
		return `/api/admin/ops/${selected.slice(OPS_PREFIX.length)}`;
	}
	return `/api/entities/${encodeURIComponent(selected)}`;
}

/**
 * Every table as the server stores it: the auth and operations tables of the
 * framework (read-only), and the unpaged rows of each app entity.
 */
export function EntitiesPage() {
	const { me } = useAuth();
	const entities = MANIFEST.entities;
	const [selected, setSelected] = useState<string>(
		`${AUTH_PREFIX}${AUTH_TABLES[0].key}`,
	);
	const [openRow, setOpenRow] = useState<Row | null>(null);
	const [creating, setCreating] = useState(false);
	const [deleting, setDeleting] = useState<Row | null>(null);
	const lastOpened = useRef<HTMLElement | null>(null);

	const framework =
		selected.startsWith(AUTH_PREFIX) || selected.startsWith(OPS_PREFIX);
	const entity = useMemo<ManifestEntity | undefined>(
		() => (framework ? undefined : entities.find((e) => e.name === selected)),
		[entities, selected, framework],
	);
	const isAdmin = !!me?.is_admin;

	const query = useQuery(
		selected,
		async () => {
			const data = await api<Row[] | { data?: Row[] }>(tablePath(selected));
			return {
				table: selected,
				rows: Array.isArray(data) ? data : (data?.data ?? []),
			};
		},
		{ enabled: isAdmin },
	);
	// The rows of the table that was selected before are not shown.
	const rows = query.data?.table === selected ? query.data.rows : null;
	const { error, loading, reload: load } = query;

	// The framework tables have no manifest entry, so the columns are the
	// keys of the rows in the order they first appear.
	const columns = useMemo(() => {
		if (!rows || rows.length === 0) {
			return entity ? ["id", ...entity.fields.map((f) => f.name)] : [];
		}
		const seen = new Set<string>();
		for (const r of rows) for (const k of Object.keys(r)) seen.add(k);
		return [...seen];
	}, [rows, entity]);

	if (!isAdmin) {
		return (
			<Page title="System tables">
				<LockedPage
					title="System tables are for admins"
					description="Sign in with an admin account to see the tables of the framework."
				/>
			</Page>
		);
	}

	const remove = async (row: Row) => {
		if (!row.id || framework) return;
		try {
			await api(`${tablePath(selected)}/${encodeURIComponent(row.id)}`, {
				method: "DELETE",
			});
			toast.success("The row is deleted");
			if (openRow?.id === row.id) setOpenRow(null);
			void load();
		} catch (err) {
			toast.error("Studio could not delete the row", {
				description: errorText(err),
			});
		}
	};

	return (
		<Page
			title="System tables"
			meta={rows ? plural(rows.length, "row") : undefined}
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
					{!framework && (
						<Button
							onClick={() => {
								setOpenRow(null);
								setCreating(true);
							}}
						>
							<Plus />
							Insert row
						</Button>
					)}
				</>
			}
		>
			<Toolbar>
				<Select
					aria-label="Table"
					className="w-full sm:w-64"
					value={selected}
					onValueChange={(next) => {
						setSelected(next);
						setOpenRow(null);
						setCreating(false);
					}}
					options={[
						{
							label: "Auth tables",
							options: AUTH_TABLES.map((t) => ({
								value: `${AUTH_PREFIX}${t.key}`,
								label: t.label,
							})),
						},
						{
							label: "Operations",
							options: OPS_TABLES.map((t) => ({
								value: `${OPS_PREFIX}${t.key}`,
								label: t.label,
							})),
						},
						...(entities.length > 0
							? [
									{
										label: "App entities",
										options: entities.map((e) => ({
											value: e.name,
											label: e.name,
										})),
									},
								]
							: []),
					]}
				/>
				{framework && <Badge variant="outline">Read-only</Badge>}
			</Toolbar>

			<div className="min-h-0 flex-1 overflow-auto">
				{error && rows === null ? (
					<ErrorState
						title="Studio could not load the table"
						message={error}
						onRetry={load}
					/>
				) : rows === null ? (
					<LoadingRows rows={10} />
				) : rows.length === 0 ? (
					<EmptyState
						title="The table has no rows"
						action={
							!framework && (
								<Button variant="outline" onClick={() => setCreating(true)}>
									<Plus />
									Insert row
								</Button>
							)
						}
					/>
				) : (
					<Table aria-label="Rows">
						<TableHeader>
							<tr>
								{columns.map((c) => (
									<TableHead key={c} className="font-mono normal-case">
										{c}
									</TableHead>
								))}
								{!framework && (
									<TableHead className="w-px px-1">
										<span className="sr-only">Actions</span>
									</TableHead>
								)}
							</tr>
						</TableHeader>
						<TableBody>
							{rows.map((row, i) => (
								<TableRow
									key={row.id ?? i}
									tabIndex={0}
									data-current={openRow !== null && openRow === row}
									className="cursor-default"
									onClick={(e) => {
										lastOpened.current = e.currentTarget;
										setCreating(false);
										setOpenRow(row);
									}}
									onKeyDown={(e) => {
										if (e.target !== e.currentTarget) return;
										if (e.key !== "Enter" && e.key !== " ") return;
										e.preventDefault();
										e.currentTarget.click();
									}}
								>
									{columns.map((c) => (
										<TableCell key={c}>
											<AutoCell
												value={row[c]}
												type={
													entity
														? entity.fields.find((f) => f.name === c)?.type
														: isTimeColumn(c, row[c])
															? "datetime"
															: undefined
												}
												isId={c === "id" && !framework}
												refEntity={
													framework ? null : refTarget(MANIFEST, entity, c)
												}
											/>
										</TableCell>
									))}
									{!framework && (
										<TableCell
											className="w-px px-1"
											onClick={(e) => e.stopPropagation()}
										>
											<Menu>
												<MenuTrigger
													aria-label="Row actions"
													className="flex size-6 cursor-default items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground data-open:bg-accent"
												>
													<MoreHorizontal className="size-3.5" />
												</MenuTrigger>
												<MenuContent align="end">
													<MenuItem onSelect={() => setOpenRow(row)}>Open</MenuItem>
													<MenuSeparator />
													<MenuItem
														destructive
														disabled={!row.id}
														onSelect={() => setDeleting(row)}
													>
														Delete
													</MenuItem>
												</MenuContent>
											</Menu>
										</TableCell>
									)}
								</TableRow>
							))}
						</TableBody>
					</Table>
				)}
			</div>

			<RowEditor
				open={creating || openRow !== null}
				mode={creating ? "create" : framework ? "view" : "edit"}
				row={creating ? null : openRow}
				entity={entity}
				entityName={
					framework
						? ([...AUTH_TABLES, ...OPS_TABLES].find(
								(t) => selected === `${AUTH_PREFIX}${t.key}` || selected === `${OPS_PREFIX}${t.key}`,
							)?.label ?? selected)
						: selected
				}
				returnFocus={() => lastOpened.current}
				onClose={() => {
					setCreating(false);
					setOpenRow(null);
				}}
				onDelete={framework ? undefined : setDeleting}
				onSaved={(saved) => {
					setCreating(false);
					setOpenRow(saved && !creating ? saved : null);
					void load();
				}}
			/>

			<ConfirmDialog
				open={deleting !== null}
				title="Delete the row?"
				description={`This deletes the ${selected} row ${shortId(String(deleting?.id ?? ""))}. You cannot undo this.`}
				confirmLabel="Delete row"
				destructive
				onCancel={() => setDeleting(null)}
				onConfirm={async () => {
					if (deleting) await remove(deleting);
					setDeleting(null);
				}}
			/>
		</Page>
	);
}
