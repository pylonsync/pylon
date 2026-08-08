import { useEffect, useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight, Loader2, Save } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError, type ManifestEntity, api } from "@/lib/pylon";

// Per-field row editor used by both the generic Entities page and the
// custom-config ResourceList page. Lives here (not co-located with
// either page) because both pages render it for the same purpose:
// click a row → editable inputs → PATCH /api/entities/<entity>/<id>.
//
// Read-only mode renders raw JSON instead — used for framework-internal
// auth/ops tables on the Entities page where edits would corrupt
// state.

export type Row = Record<string, unknown> & { id?: string };

export function RowEditor({
	row,
	entity,
	entityName,
	readOnly,
	onClose,
	onSaved,
}: {
	row: Row | null;
	entity: ManifestEntity | undefined;
	entityName: string;
	readOnly?: boolean;
	onClose: () => void;
	onSaved: () => void;
}) {
	const [draft, setDraft] = useState<Record<string, unknown>>({});
	const [showJson, setShowJson] = useState(false);
	const [saving, setSaving] = useState(false);

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
			await api(`/api/entities/${entityName}/${row.id}`, {
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

	const editable = !readOnly && entity !== undefined;

	return (
		<Dialog open onOpenChange={(o) => !o && onClose()}>
			<DialogContent className="sm:max-w-[640px]">
				<DialogHeader>
					<DialogTitle>{editable ? "Edit row" : "Inspect row"}</DialogTitle>
					<DialogDescription className="font-mono text-xs">
						{(row.id as string) ?? "—"}
					</DialogDescription>
				</DialogHeader>

				{editable ? (
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
						{editable ? "Cancel" : "Close"}
					</Button>
					{editable && (
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
						onChange(`${v}:00Z`);
					}}
				/>
			</div>
		);
	}

	if (type === "json") {
		return (
			<JsonFieldEditor
				name={name}
				value={value}
				onChange={onChange}
				labelEl={labelEl}
			/>
		);
	}

	// string, richtext, id(X), or anything unknown.
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

// JSON field editor: a monospace textarea over the pretty-printed value.
// Local raw-text state lets the operator type through invalid
// intermediate states; only a successful parse propagates via onChange,
// so the row payload always carries a real JSON value (never a string
// of JSON). Exported for Entities.tsx, which mirrors this editor set.
export function JsonFieldEditor({
	name,
	value,
	onChange,
	labelEl,
}: {
	name: string;
	value: unknown;
	onChange: (v: unknown) => void;
	labelEl: ReactNode;
}) {
	const [raw, setRaw] = useState<string>(() =>
		value === undefined ? "" : JSON.stringify(value, null, 2),
	);
	const [invalid, setInvalid] = useState(false);
	return (
		<div className="space-y-1.5">
			{labelEl}
			<textarea
				id={`field-${name}`}
				value={raw}
				rows={4}
				spellCheck={false}
				className="w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-xs shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
				onChange={(e) => {
					const t = e.target.value;
					setRaw(t);
					if (t.trim() === "") {
						setInvalid(false);
						onChange(null);
						return;
					}
					try {
						onChange(JSON.parse(t));
						setInvalid(false);
					} catch {
						setInvalid(true);
					}
				}}
			/>
			{invalid ? (
				<p className="text-[11px] text-destructive">
					Invalid JSON — the last valid value is what saves.
				</p>
			) : null}
		</div>
	);
}
