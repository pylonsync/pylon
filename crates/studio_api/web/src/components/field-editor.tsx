import { useState, type ReactNode } from "react";

import { RefCell } from "@/components/renderers";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";

export interface FieldEditorProps {
	name: string;
	/** The manifest type of the field. */
	type: string;
	optional: boolean;
	value: unknown;
	onChange: (value: unknown) => void;
	/** The entity the field points to, when the field is a reference. */
	refEntity?: string | null;
	/** The display value of the row the field points to. */
	refLabel?: string | null;
}

/** The label row of a field: the name on the left, the type on the right. */
export function FieldLabel({
	name,
	type,
	optional,
	htmlFor,
}: {
	name: string;
	type: string;
	optional?: boolean;
	htmlFor: string;
}) {
	return (
		<div className="flex items-baseline justify-between gap-2">
			<Label htmlFor={htmlFor} className="font-mono">
				{name}
			</Label>
			<span className="font-mono text-[11px] text-subtle-foreground">
				{type}
				{optional && ", optional"}
			</span>
		</div>
	);
}

/** The editor for one field of a row. The control depends on the manifest type. */
export function FieldEditor({
	name,
	type,
	optional,
	value,
	onChange,
	refEntity,
	refLabel,
}: FieldEditorProps) {
	const [initialMultiline] = useState(() => {
		const t =
			value === null || value === undefined
				? ""
				: typeof value === "string"
					? value
					: JSON.stringify(value);
		return t.length > 80 || t.includes("\n");
	});
	const id = `field-${name}`;
	const label = (
		<FieldLabel name={name} type={type} optional={optional} htmlFor={id} />
	);

	if (type === "bool") {
		return (
			<div className="space-y-1">
				{label}
				<div className="flex h-7 items-center gap-2">
					<Switch
						id={id}
						checked={value === true}
						onCheckedChange={(checked) => onChange(checked)}
					/>
					<span className="font-mono text-xs text-muted-foreground">
						{value === true ? "true" : value === false ? "false" : "not set"}
					</span>
				</div>
			</div>
		);
	}

	if (type === "int" || type === "float") {
		return (
			<div className="space-y-1">
				{label}
				<Input
					id={id}
					name={name}
					type="number"
					step={type === "int" ? "1" : "any"}
					className="tabular-nums"
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
		// The stored value is ISO 8601 in UTC. The control edits the same UTC
		// clock time, to the minute.
		const local =
			typeof value === "string"
				? (/^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2})/.exec(value)?.[1] ?? "")
				: "";
		return (
			<div className="space-y-1">
				{label}
				<Input
					id={id}
					name={name}
					type="datetime-local"
					aria-description="The time is in UTC."
					value={local}
					onChange={(e) => {
						const v = e.target.value;
						if (!v) {
							onChange(optional ? null : "");
							return;
						}
						onChange(`${v}:00Z`);
					}}
				/>
				<p className="text-[11px] text-subtle-foreground">UTC</p>
			</div>
		);
	}

	if (type === "json") {
		return <JsonFieldEditor id={id} value={value} onChange={onChange} label={label} />;
	}

	const text =
		value === null || value === undefined
			? ""
			: typeof value === "string"
				? value
				: JSON.stringify(value);
	const onText = (v: string) => onChange(v === "" && optional ? null : v);
	// Chosen from the value the editor opened with, so the control does not
	// change from an input to a text area (and lose focus) while the operator
	// types past a length or a line break.
	const multiline = initialMultiline;

	if (type === "richtext" || multiline) {
		return (
			<div className="space-y-1">
				{label}
				<Textarea
					id={id}
					name={name}
					rows={4}
					value={text}
					onChange={(e) => onText(e.target.value)}
				/>
			</div>
		);
	}

	return (
		<div className="space-y-1">
			{label}
			<Input
				id={id}
				name={name}
				type="text"
				className={refEntity ? "font-mono text-xs" : undefined}
				value={text}
				placeholder={optional && value === null ? "null" : undefined}
				onChange={(e) => onText(e.target.value)}
			/>
			{refEntity && typeof value === "string" && value && (
				<p className="truncate text-xs text-muted-foreground">
					{refEntity}: <RefCell entity={refEntity} id={value} label={refLabel} />
				</p>
			)}
		</div>
	);
}

/**
 * A JSON value as text. The operator can type through text that does not
 * parse. `onChange` runs only for text that parses, so the row always holds a
 * JSON value and never a string of JSON.
 */
export function JsonFieldEditor({
	id,
	value,
	onChange,
	label,
	rows = 5,
}: {
	id: string;
	value: unknown;
	onChange: (value: unknown) => void;
	label?: ReactNode;
	rows?: number;
}) {
	const [raw, setRaw] = useState<string>(() =>
		value === undefined ? "" : JSON.stringify(value, null, 2),
	);
	const [invalid, setInvalid] = useState(false);
	return (
		<div className="space-y-1">
			{label}
			<Textarea
				id={id}
				value={raw}
				rows={rows}
				spellCheck={false}
				aria-invalid={invalid || undefined}
				className="font-mono text-xs leading-relaxed aria-invalid:border-destructive"
				onChange={(e) => {
					const text = e.target.value;
					setRaw(text);
					if (text.trim() === "") {
						setInvalid(false);
						onChange(null);
						return;
					}
					try {
						onChange(JSON.parse(text));
						setInvalid(false);
					} catch {
						setInvalid(true);
					}
				}}
			/>
			{invalid && (
				<p className="text-[11px] text-destructive">
					This text is not valid JSON. Studio saves the last valid value.
				</p>
			)}
		</div>
	);
}
