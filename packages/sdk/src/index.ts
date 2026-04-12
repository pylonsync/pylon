// ---------------------------------------------------------------------------
// Route modes
// ---------------------------------------------------------------------------

export type RouteMode = "static" | "server" | "live";

// ---------------------------------------------------------------------------
// Field types
// ---------------------------------------------------------------------------

export type FieldType =
  | "string"
  | "int"
  | "float"
  | "bool"
  | "datetime"
  | "richtext"
  | `id(${string})`;

// ---------------------------------------------------------------------------
// Field builder
// ---------------------------------------------------------------------------

export interface FieldDefinition {
  type: FieldType;
  optional: boolean;
  unique: boolean;
}

interface FieldBuilder {
  readonly _def: FieldDefinition;
  optional(): FieldBuilder;
  unique(): FieldBuilder;
}

function createFieldBuilder(type: FieldType): FieldBuilder {
  return buildField({ type, optional: false, unique: false });
}

function buildField(def: FieldDefinition): FieldBuilder {
  return {
    _def: def,
    optional() {
      return buildField({ ...def, optional: true });
    },
    unique() {
      return buildField({ ...def, unique: true });
    },
  };
}

export const field = {
  string: () => createFieldBuilder("string"),
  int: () => createFieldBuilder("int"),
  float: () => createFieldBuilder("float"),
  bool: () => createFieldBuilder("bool"),
  datetime: () => createFieldBuilder("datetime"),
  richtext: () => createFieldBuilder("richtext"),
  id: (target: string) => createFieldBuilder(`id(${target})`),
};

// ---------------------------------------------------------------------------
// Entity builder
// ---------------------------------------------------------------------------

export interface IndexDefinition {
  name: string;
  fields: string[];
  unique: boolean;
}

export interface EntityDefinition {
  name: string;
  fields: Record<string, FieldBuilder>;
  indexes?: IndexDefinition[];
}

export function entity(
  name: string,
  fields: Record<string, FieldBuilder>,
  options?: { indexes?: IndexDefinition[] }
): EntityDefinition {
  return { name, fields, indexes: options?.indexes };
}

// ---------------------------------------------------------------------------
// Route definition
// ---------------------------------------------------------------------------

export interface RouteDefinition {
  path: string;
  mode: RouteMode;
}

export function defineRoute(route: RouteDefinition): RouteDefinition {
  return route;
}

// ---------------------------------------------------------------------------
// Manifest generation
// ---------------------------------------------------------------------------

export interface ManifestField {
  name: string;
  type: FieldType;
  optional: boolean;
  unique: boolean;
}

export interface ManifestIndex {
  name: string;
  fields: string[];
  unique: boolean;
}

export interface ManifestEntity {
  name: string;
  fields: ManifestField[];
  indexes: ManifestIndex[];
}

export interface ManifestRoute {
  path: string;
  mode: string;
}

export interface AppManifest {
  name: string;
  version: string;
  entities: ManifestEntity[];
  routes: ManifestRoute[];
}

export function entitiesToManifest(
  entities: EntityDefinition[]
): ManifestEntity[] {
  return entities.map((e) => ({
    name: e.name,
    fields: Object.entries(e.fields).map(([name, fb]) => ({
      name,
      type: fb._def.type,
      optional: fb._def.optional,
      unique: fb._def.unique,
    })),
    indexes: (e.indexes ?? []).map((idx) => ({
      name: idx.name,
      fields: idx.fields,
      unique: idx.unique,
    })),
  }));
}

export function routesToManifest(routes: RouteDefinition[]): ManifestRoute[] {
  return routes.map((r) => ({ path: r.path, mode: r.mode }));
}

export function buildManifest(options: {
  name: string;
  version: string;
  entities: EntityDefinition[];
  routes: RouteDefinition[];
}): AppManifest {
  return {
    name: options.name,
    version: options.version,
    entities: entitiesToManifest(options.entities),
    routes: routesToManifest(options.routes),
  };
}
