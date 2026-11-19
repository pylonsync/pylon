// References between entities.
//
// The manifest gives a reference field the type `id(<Entity>)`. A table shows
// such a field as the display value of the row it points to, and links to
// that row.

import { api, type Manifest, type ManifestEntity } from "@/lib/pylon";

type Row = Record<string, unknown>;

const ID_TYPE = /^id\(\s*([A-Za-z_][\w]*)\s*\)$/;

/** `"id(Company)"` gives `"Company"`. Any other type gives `null`. */
export function refTargetOfType(type: string): string | null {
	return ID_TYPE.exec(type)?.[1] ?? null;
}

/**
 * The entity a field points to, or `null` when the field is not a reference.
 *
 * The manifest type decides. A field the manifest does not type as a
 * reference still counts when its name is `<entity>Id` or `<entity>_id` and
 * an entity of that name exists.
 */
export function refTarget(
	manifest: Manifest,
	entity: ManifestEntity | undefined,
	fieldName: string,
): string | null {
	const field = entity?.fields.find((f) => f.name === fieldName);
	if (field) {
		const typed = refTargetOfType(field.type);
		if (typed) return manifest.entities.some((e) => e.name === typed) ? typed : null;
	}
	const stem = /^(.+?)_?[iI]d$/.exec(fieldName)?.[1];
	if (!stem) return null;
	const match = manifest.entities.find(
		(e) => e.name.toLowerCase() === stem.toLowerCase(),
	);
	return match?.name ?? null;
}

const DISPLAY_FIELDS = [
	"name",
	"title",
	"label",
	"displayName",
	"display_name",
	"email",
	"username",
	"slug",
];

/** The field that names a row of this entity for a person, if there is one. */
export function displayField(entity: ManifestEntity): string | null {
	const strings = entity.fields.filter((f) => f.type === "string");
	for (const name of DISPLAY_FIELDS) {
		if (strings.some((f) => f.name === name)) return name;
	}
	return strings.find((f) => !f.optional)?.name ?? null;
}

/** The display value of a row, or `null` when the row has none. */
export function rowLabel(entity: ManifestEntity, row: Row): string | null {
	const field = displayField(entity);
	if (!field) return null;
	const v = row[field];
	return typeof v === "string" && v.trim() ? v.trim() : null;
}

// ---------------------------------------------------------------------------
// Label lookup
// ---------------------------------------------------------------------------

/** Up to this many missing rows are fetched one by one. */
const SINGLE_FETCH_LIMIT = 12;
/** The largest page the list endpoint returns. */
const BULK_PAGE_SIZE = 1000;

// entity -> row id -> label. `null` records a row with no label or a row the
// server did not return, so the same id is not requested again.
const cache = new Map<string, Map<string, string | null>>();

function cacheFor(entity: string): Map<string, string | null> {
	let m = cache.get(entity);
	if (!m) {
		m = new Map();
		cache.set(entity, m);
	}
	return m;
}

/** Forget the labels of one entity. Call it after a row of the entity changes. */
export function forgetLabels(entity: string): void {
	cache.delete(entity);
}

/**
 * Look up the display values of rows of `target`.
 *
 * The cost has a fixed limit for each call: a few single-row requests, or one
 * request for the first page of the entity. Ids that neither covers get no
 * label and the table shows the id.
 */
export async function loadLabels(
	manifest: Manifest,
	target: string,
	ids: string[],
): Promise<Map<string, string | null>> {
	const entity = manifest.entities.find((e) => e.name === target);
	const known = cacheFor(target);
	if (!entity || !displayField(entity)) return known;

	const missing = [...new Set(ids)].filter((id) => !known.has(id));
	if (missing.length === 0) return known;

	const path = `/api/entities/${encodeURIComponent(target)}`;
	if (missing.length <= SINGLE_FETCH_LIMIT) {
		await Promise.all(
			missing.map(async (id) => {
				try {
					const row = await api<Row>(`${path}/${encodeURIComponent(id)}`);
					known.set(id, row ? rowLabel(entity, row) : null);
				} catch {
					known.set(id, null);
				}
			}),
		);
		return known;
	}

	try {
		const page = await api<Row[] | { data?: Row[] }>(
			`${path}?page=1&per_page=${BULK_PAGE_SIZE}`,
		);
		const rows = Array.isArray(page) ? page : (page?.data ?? []);
		for (const row of rows) {
			if (typeof row.id === "string") known.set(row.id, rowLabel(entity, row));
		}
	} catch {
		// The ids stay unresolved and are marked below.
	}
	for (const id of missing) if (!known.has(id)) known.set(id, null);
	return known;
}
