// Row action plumbing: turning a declarative `rowActions` entry plus the
// row it was clicked on into a function call, and turning what comes back
// into something to show.
//
// Kept out of the component so the interpolation rules are unit-testable —
// they're the part with edge cases (missing fields, non-string values,
// nested paths), and getting one wrong silently sends the wrong argument
// to a mutation.

import type { RowAction } from "./studio-config";

export type RowLike = Record<string, unknown>;

/** `{row.<path>}` — the whole value, nothing around it. */
const WHOLE_PLACEHOLDER = /^\{row\.([^{}]+)\}$/;
/** Every `{row.<path>}` occurrence, for string interpolation. */
const ANY_PLACEHOLDER = /\{row\.([^{}]+)\}/g;

/**
 * Read a dot path out of a row. `getRowField(r, "a.b")` reads `r.a.b`;
 * returns `undefined` for any missing segment rather than throwing.
 */
export function getRowField(row: RowLike, field: string): unknown {
	if (!field.includes(".")) return row[field];
	let v: unknown = row;
	for (const seg of field.split(".")) {
		if (v && typeof v === "object" && seg in (v as RowLike)) {
			v = (v as RowLike)[seg];
		} else return undefined;
	}
	return v;
}

/**
 * Build the argument object for `kind: "action"`.
 *
 * With no `input`, sends `{ id: <row id> }` — the overwhelmingly common
 * case, and the one that makes a bare `{ id, label, kind, action }` work.
 *
 * With `input`, every string is interpolated. A value that is *exactly*
 * one placeholder keeps the row value's JSON type, so `"{row.count}"`
 * sends the number `3` and not the string `"3"`. A placeholder embedded
 * in a larger string stringifies, and a missing field becomes `""` there
 * (vs. `null` when it's the whole value) — an interpolated string with a
 * literal "undefined" in it is never what anyone meant.
 */
export function buildActionInput(
	action: RowAction,
	row: RowLike,
): Record<string, unknown> {
	if (!action.input) return { id: row.id ?? null };
	const out: Record<string, unknown> = {};
	for (const [key, value] of Object.entries(action.input)) {
		out[key] = interpolate(value, row);
	}
	return out;
}

function interpolate(value: unknown, row: RowLike): unknown {
	if (typeof value === "string") {
		const whole = WHOLE_PLACEHOLDER.exec(value);
		if (whole) {
			const v = getRowField(row, whole[1]);
			return v === undefined ? null : v;
		}
		return value.replace(ANY_PLACEHOLDER, (_, path: string) => {
			const v = getRowField(row, path);
			return v == null ? "" : String(v);
		});
	}
	if (Array.isArray(value)) return value.map((v) => interpolate(v, row));
	if (value && typeof value === "object") {
		const out: Record<string, unknown> = {};
		for (const [k, v] of Object.entries(value as RowLike)) {
			out[k] = interpolate(v, row);
		}
		return out;
	}
	return value;
}

/** Narrow a function's return value down to `resultField`, if set. */
export function pickResultValue(value: unknown, field?: string): unknown {
	if (!field) return value;
	if (!value || typeof value !== "object") return undefined;
	return getRowField(value as RowLike, field);
}

/**
 * Render a result for a toast or the clipboard. Strings pass through
 * unquoted — a generated URL should be pasteable, not `"https://…"`.
 */
export function resultToText(value: unknown): string {
	if (value == null) return "";
	if (typeof value === "string") return value;
	if (typeof value === "object") return JSON.stringify(value, null, 2);
	return String(value);
}

/**
 * Copy to the clipboard, reporting whether it worked.
 *
 * `navigator.clipboard` is unavailable outside a secure context, which
 * includes Studio served over plain http on a LAN address. When it isn't
 * there, the caller falls back to showing the value in a dialog the user
 * can select — same outcome, one more click.
 */
export async function copyToClipboard(text: string): Promise<boolean> {
	try {
		if (!navigator.clipboard?.writeText) return false;
		await navigator.clipboard.writeText(text);
		return true;
	} catch {
		return false;
	}
}
