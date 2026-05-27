"use client";

import {
	type FormEvent,
	type ReactNode,
	useEffect,
	useState,
} from "react";
import { db, type Row } from "@pylonsync/react";
import { cn } from "../lib/cn";

export interface EntityFormField {
	/** Field key on the row. */
	field: string;
	/** Display label. Defaults to a humanized `field` name. */
	label?: ReactNode;
	/** Input type hint. Default: "text". */
	type?:
		| "text"
		| "email"
		| "url"
		| "password"
		| "number"
		| "textarea"
		| "checkbox"
		| "select";
	/** For `type: "select"`. */
	options?: Array<{ value: string; label?: string }>;
	/** Required flag. */
	required?: boolean;
	/** Placeholder. */
	placeholder?: string;
	/** Helper text shown under the input. */
	help?: ReactNode;
	/** Default value when creating. */
	defaultValue?: unknown;
}

export interface EntityFormProps<T extends Row = Row> {
	/** Entity to write into. */
	entity: string;
	/** Field descriptors. */
	fields: EntityFormField[];
	/**
	 * If set, the form edits this existing row. Omit to create a new
	 * row (the default mode).
	 */
	row?: T;
	/** Override the submit label. */
	submitLabel?: ReactNode;
	/** Optional cancel button — calls this handler when clicked. */
	onCancel?: () => void;
	/** Called after a successful create/update. */
	onSubmitted?: (row: Row) => void;
	/** Headline shown above the form. */
	title?: ReactNode;
	className?: string;
}

/**
 * Drop-in create / edit form for any entity. Uses `db.useEntity` for
 * optimistic CRUD — the local replica gets the change immediately and
 * the server reconciles via the sync push. No `onSubmit` boilerplate.
 *
 * ```tsx
 * <EntityForm
 *   entity="Post"
 *   fields={[
 *     { field: "title", required: true },
 *     { field: "body", type: "textarea" },
 *     { field: "status", type: "select", options: [
 *       { value: "draft" }, { value: "published" },
 *     ]},
 *   ]}
 *   onSubmitted={(row) => router.push(`/posts/${row.id}`)}
 * />
 * ```
 */
export function EntityForm<T extends Row = Row>({
	entity,
	fields,
	row,
	submitLabel,
	onCancel,
	onSubmitted,
	title,
	className,
}: EntityFormProps<T>) {
	const mutation = db.useEntity(entity);
	const [values, setValues] = useState<Record<string, unknown>>(() =>
		initialValues(fields, row),
	);
	const [error, setError] = useState<string | null>(null);
	const [pending, setPending] = useState(false);

	useEffect(() => {
		setValues(initialValues(fields, row));
	}, [fields, row]);

	async function onSubmit(e: FormEvent) {
		e.preventDefault();
		setError(null);
		setPending(true);
		try {
			let result: Row;
			if (row?.id) {
				await mutation.update(String(row.id), values);
				result = { ...row, ...values } as Row;
			} else {
				// `insert` returns the new row id; merge it back into the
				// values so consumers see the same shape they'd get from
				// a fresh fetch.
				const id = await mutation.insert(values as Row);
				result = { ...values, id } as Row;
			}
			onSubmitted?.(result);
		} catch (err) {
			setError(err instanceof Error ? err.message : "Save failed.");
		} finally {
			setPending(false);
		}
	}

	return (
		<form
			onSubmit={onSubmit}
			className={cn(
				"mx-auto w-full max-w-md space-y-4 rounded-xl border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] p-6 shadow-sm",
				className,
			)}
		>
			{title ? (
				<h3 className="text-base font-semibold tracking-tight text-[var(--pylon-ink,#0a0a0a)]">
					{title}
				</h3>
			) : null}
			{fields.map((f) => (
				<FieldRenderer
					key={f.field}
					field={f}
					value={values[f.field]}
					onChange={(v) =>
						setValues((prev) => ({ ...prev, [f.field]: v }))
					}
				/>
			))}
			<div className="flex items-center gap-2">
				<button
					type="submit"
					disabled={pending}
					className="rounded-md bg-[var(--pylon-ink,#0a0a0a)] px-3 py-2 text-sm font-medium text-[var(--pylon-paper,#ffffff)] transition-opacity hover:opacity-90 disabled:opacity-60"
				>
					{pending ? "…" : (submitLabel ?? (row ? "Save" : "Create"))}
				</button>
				{onCancel ? (
					<button
						type="button"
						onClick={onCancel}
						className="text-sm text-[var(--pylon-ink-2,#52525b)] hover:underline"
					>
						Cancel
					</button>
				) : null}
			</div>
			{error ? (
				<p className="rounded-md border border-[var(--pylon-error-rule,#fecaca)] bg-[var(--pylon-error-bg,#fef2f2)] px-3 py-2 text-xs text-[var(--pylon-error-ink,#b91c1c)]">
					{error}
				</p>
			) : null}
		</form>
	);
}

function FieldRenderer({
	field,
	value,
	onChange,
}: {
	field: EntityFormField;
	value: unknown;
	onChange: (v: unknown) => void;
}) {
	const label = (
		<span className="text-xs font-medium text-[var(--pylon-ink-2,#52525b)]">
			{field.label ?? humanize(field.field)}
			{field.required ? (
				<span className="ml-1 text-[var(--pylon-error-ink,#b91c1c)]">*</span>
			) : null}
		</span>
	);
	const inputCls =
		"w-full rounded-md border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] px-3 py-2 text-sm text-[var(--pylon-ink,#0a0a0a)] placeholder:text-[var(--pylon-ink-3,#a1a1aa)] focus:border-[var(--pylon-ink,#0a0a0a)] focus:outline-none";

	const help = field.help ? (
		<span className="text-[11px] text-[var(--pylon-ink-3,#71717a)]">
			{field.help}
		</span>
	) : null;

	if (field.type === "textarea") {
		return (
			<label className="block space-y-1.5">
				{label}
				<textarea
					value={(value as string) ?? ""}
					onChange={(e) => onChange(e.target.value)}
					required={field.required}
					placeholder={field.placeholder}
					rows={4}
					className={inputCls}
				/>
				{help}
			</label>
		);
	}

	if (field.type === "checkbox") {
		return (
			<label className="flex items-start gap-2">
				<input
					type="checkbox"
					checked={Boolean(value)}
					onChange={(e) => onChange(e.target.checked)}
					className="mt-0.5 h-4 w-4 rounded border-[var(--pylon-rule,#e5e7eb)]"
				/>
				<span className="space-y-0.5">
					{label}
					{help}
				</span>
			</label>
		);
	}

	if (field.type === "select") {
		return (
			<label className="block space-y-1.5">
				{label}
				<select
					value={(value as string) ?? ""}
					onChange={(e) => onChange(e.target.value)}
					required={field.required}
					className={inputCls}
				>
					<option value="" disabled hidden>
						Select…
					</option>
					{(field.options ?? []).map((opt) => (
						<option key={opt.value} value={opt.value}>
							{opt.label ?? opt.value}
						</option>
					))}
				</select>
				{help}
			</label>
		);
	}

	return (
		<label className="block space-y-1.5">
			{label}
			<input
				type={field.type ?? "text"}
				value={(value as string | number | undefined) ?? ""}
				onChange={(e) =>
					onChange(
						field.type === "number"
							? e.target.value === ""
								? null
								: Number(e.target.value)
							: e.target.value,
					)
				}
				required={field.required}
				placeholder={field.placeholder}
				className={inputCls}
			/>
			{help}
		</label>
	);
}

function initialValues(
	fields: EntityFormField[],
	row: Row | undefined,
): Record<string, unknown> {
	const out: Record<string, unknown> = {};
	for (const f of fields) {
		if (row && row[f.field] !== undefined) {
			out[f.field] = row[f.field];
		} else if (f.defaultValue !== undefined) {
			out[f.field] = f.defaultValue;
		} else if (f.type === "checkbox") {
			out[f.field] = false;
		} else {
			out[f.field] = "";
		}
	}
	return out;
}

function humanize(field: string): string {
	return field
		.replace(/([A-Z])/g, " $1")
		.replace(/^./, (c) => c.toUpperCase())
		.replace(/_/g, " ")
		.trim();
}
