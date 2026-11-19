import { useRef, useState } from "react";
import { Trash2, X } from "lucide-react";

import { CodeBlock } from "@/components/code-block";
import { CopyButton } from "@/components/copy-button";
import { FieldEditor, JsonFieldEditor } from "@/components/field-editor";
import { Button } from "@/components/ui/button";
import {
	Sheet,
	SheetBody,
	SheetClose,
	SheetContent,
	SheetDescription,
	SheetFooter,
	SheetHeader,
	SheetTitle,
} from "@/components/ui/sheet";
import { Tabs, TabsList, TabsPanel, TabsTab } from "@/components/ui/tabs";
import { toast } from "@/lib/toast";
import { useEscapeOutside } from "@/hooks/use-escape";
import { errorText, plural } from "@/lib/format";
import { MANIFEST, type ManifestEntity, api } from "@/lib/pylon";
import { refTarget, rowLabel } from "@/lib/refs";

export type Row = Record<string, unknown> & { id?: string };

export type RowEditorMode = "edit" | "create" | "view";

export interface RowEditorProps {
	/** Whether the panel is open. */
	open: boolean;
	/**
	 * `edit` changes a row, `create` inserts a row, and `view` shows a row
	 * that Studio must not change.
	 */
	mode: RowEditorMode;
	/** The row to edit or view. `create` ignores it. */
	row: Row | null;
	/** The manifest entry of the entity. Framework tables have none. */
	entity: ManifestEntity | undefined;
	/** The name in the API path and in the panel title. */
	entityName: string;
	/** Display values of referenced rows, by entity and id. */
	refLabels?: (entity: string, id: string) => string | null | undefined;
	onClose: () => void;
	/** Runs after a save, with the row from the server when it returns one. */
	onSaved: (row: Row | null) => void;
	/** Shows a Delete button in `edit` mode and runs when it is pressed. */
	onDelete?: (row: Row) => void;
	/** The element that gets focus after the panel closes. */
	returnFocus?: () => HTMLElement | null;
}

/**
 * The row panel: a sheet on the right edge with one editor for each field.
 *
 * The sheet is not modal. The list behind it stays usable, so the operator
 * can open the next row without closing the panel. Escape closes the panel.
 */
export function RowEditor({
	open,
	mode,
	row,
	entity,
	entityName,
	refLabels,
	onClose,
	onSaved,
	onDelete,
	returnFocus,
}: RowEditorProps) {
	// The last content stays rendered while the sheet animates out.
	const [shown, setShown] = useState({ mode, row });
	if (open && (shown.mode !== mode || shown.row !== row)) {
		setShown({ mode, row });
	}
	const panel = useRef<HTMLDivElement>(null);

	useEscapeOutside(open, panel, onClose);

	const current = shown.row;
	const label =
		shown.mode === "create"
			? `New ${entityName} row`
			: ((entity && current && rowLabel(entity, current)) ?? `${entityName} row`);

	return (
		<Sheet
			open={open}
			modal={false}
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
		>
			<SheetContent
				ref={panel}
				side="right"
				backdrop={false}
				initialFocus="content"
				finalFocus={returnFocus}
				className="w-[min(460px,100vw)]"
			>
				<SheetHeader className="justify-between">
					<div className="flex min-w-0 flex-col">
						<SheetTitle>{label}</SheetTitle>
						{shown.mode !== "create" && typeof current?.id === "string" && (
							<SheetDescription className="flex items-center gap-1 font-mono text-[11px]">
								<span className="truncate">{current.id}</span>
								<CopyButton value={current.id} label="Copy the id" />
							</SheetDescription>
						)}
					</div>
					<SheetClose
						aria-label="Close"
						className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
					>
						<X className="size-3.5" />
					</SheetClose>
				</SheetHeader>
				{shown.mode === "view" || !entity ? (
					<>
						<SheetBody className="p-3">
							<CodeBlock value={JSON.stringify(current ?? {}, null, 2)} />
						</SheetBody>
						<SheetFooter>
							<Button variant="outline" onClick={onClose}>
								Close
							</Button>
						</SheetFooter>
					</>
				) : (
					<RowForm
						key={`${shown.mode}:${current?.id ?? ""}`}
						mode={shown.mode}
						row={current}
						entity={entity}
						entityName={entityName}
						refLabels={refLabels}
						onClose={onClose}
						onSaved={onSaved}
						onDelete={onDelete}
					/>
				)}
			</SheetContent>
		</Sheet>
	);
}

function RowForm({
	mode,
	row,
	entity,
	entityName,
	refLabels,
	onClose,
	onSaved,
	onDelete,
}: {
	mode: "edit" | "create";
	row: Row | null;
	entity: ManifestEntity;
	entityName: string;
	refLabels?: (entity: string, id: string) => string | null | undefined;
	onClose: () => void;
	onSaved: (row: Row | null) => void;
	onDelete?: (row: Row) => void;
}) {
	const base: Row = mode === "edit" && row ? row : {};
	// Only the fields the operator changed. A field that is not in `edits`
	// shows the current server value, so a newer value from another writer
	// replaces the old one on screen and is never sent back.
	const [edits, setEdits] = useState<Record<string, unknown>>({});
	const draft: Record<string, unknown> = { ...base, ...edits };
	const [tab, setTab] = useState<string>("fields");

	const fields = entity.fields.filter(
		(f) => f.name !== "id" && !f.name.startsWith("_"),
	);
	const dirtyKeys = Object.keys(edits).filter(
		(k) =>
			edits[k] !== undefined &&
			JSON.stringify(base[k]) !== JSON.stringify(edits[k]),
	);
	// An insert may send an empty object: every field can be optional or
	// have a default. An update needs a changed field.
	const canSave = mode === "create" || dirtyKeys.length > 0;
	const path = `/api/entities/${encodeURIComponent(entityName)}`;

	const save = async () => {
		const body: Record<string, unknown> = {};
		for (const k of dirtyKeys) body[k] = edits[k];
		try {
			if (mode === "create") {
				const created = await api<Row | null>(path, {
					method: "POST",
					body: JSON.stringify(body),
				});
				toast.success(`The ${entityName} row is inserted`);
				onSaved(created);
			} else if (row?.id) {
				const updated = await api<Row | null>(
					`${path}/${encodeURIComponent(row.id)}`,
					{ method: "PATCH", body: JSON.stringify(body) },
				);
				toast.success(
					`${plural(dirtyKeys.length, "field")} saved`,
				);
				onSaved(updated);
			}
		} catch (err) {
			toast.error(
				mode === "create"
					? "Studio could not insert the row"
					: "Studio could not save the row",
				{ description: errorText(err) },
			);
		}
	};

	return (
		<>
			<Tabs
				value={tab}
				onValueChange={(next) => next && setTab(next)}
				className="min-h-0 flex-1"
			>
				<TabsList aria-label="Row views">
					<TabsTab value="fields">Fields</TabsTab>
					<TabsTab value="json">JSON</TabsTab>
				</TabsList>
				<TabsPanel value="fields" className="overflow-y-auto">
					<form
						className="space-y-3 p-3"
						onSubmit={(e) => {
							e.preventDefault();
							if (canSave) void save();
						}}
					>
						{fields.map((f) => {
							const target = refTarget(MANIFEST, entity, f.name);
							const value = draft[f.name];
							return (
								<FieldEditor
									key={f.name}
									name={f.name}
									type={f.type}
									optional={Boolean(f.optional)}
									value={value}
									refEntity={target}
									refLabel={
										target && typeof value === "string"
											? refLabels?.(target, value)
											: undefined
									}
									onChange={(v) => setEdits((prev) => ({ ...prev, [f.name]: v }))}
								/>
							);
						})}
						{fields.length === 0 && (
							<p className="text-xs text-muted-foreground">
								This entity has no fields to edit.
							</p>
						)}
						{/* Enter in a field submits the form. */}
						<button type="submit" hidden />
					</form>
				</TabsPanel>
				<TabsPanel value="json" className="overflow-y-auto p-3">
					{mode === "create" ? (
						<JsonFieldEditor
							id="row-json"
							rows={16}
							value={edits}
							label={
								<p className="text-xs text-muted-foreground">
									The JSON object that Studio sends to the server.
								</p>
							}
							onChange={(v) => {
								if (v && typeof v === "object" && !Array.isArray(v)) {
									setEdits(v as Record<string, unknown>);
								} else if (v === null) {
									setEdits({});
								}
							}}
						/>
					) : (
						<CodeBlock value={JSON.stringify(draft, null, 2)} />
					)}
				</TabsPanel>
			</Tabs>
			<SheetFooter>
				{mode === "edit" && row && onDelete && (
					<Button
						variant="ghost"
						className="mr-auto text-destructive hover:text-destructive"
						onClick={() => onDelete(row)}
					>
						<Trash2 />
						Delete
					</Button>
				)}
				<Button variant="outline" onClick={onClose}>
					Cancel
				</Button>
				<Button disabled={!canSave} onClick={save}>
					{mode === "create"
						? "Insert row"
						: dirtyKeys.length > 0
							? `Save ${plural(dirtyKeys.length, "change")}`
							: "Save"}
				</Button>
			</SheetFooter>
		</>
	);
}
