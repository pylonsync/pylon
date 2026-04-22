/**
 * Argument validators for function definitions.
 *
 * These serve double duty:
 * 1. Runtime validation — reject bad input before the handler runs.
 * 2. Type inference — TypeScript infers handler arg types from validators.
 *
 * @example
 * ```typescript
 * import { mutation, v } from "@statecraft/functions";
 *
 * export default mutation({
 *   args: {
 *     name: v.string(),
 *     age: v.optional(v.number()),
 *     tags: v.array(v.string()),
 *   },
 *   async handler(ctx, args) {
 *     // args is typed as { name: string, age?: number, tags: string[] }
 *   },
 * });
 * ```
 */

import type { Validator } from "./types";

function validator(type: string, extra?: Partial<Validator>): Validator {
  return { type, ...extra };
}

export const v = {
  /** String value. */
  string: (): Validator => validator("string"),

  /** Number (float64). */
  number: (): Validator => validator("number"),

  /** Integer. */
  int: (): Validator => validator("int"),

  /** Boolean. */
  boolean: (): Validator => validator("boolean"),

  /** ID reference to another entity. */
  id: (table: string): Validator => validator("id", { table }),

  /** Null value. */
  null: (): Validator => validator("null"),

  /** Array of values. */
  array: (items: Validator): Validator => validator("array", { items }),

  /** Object with typed fields. */
  object: (fields: Record<string, Validator>): Validator =>
    validator("object", { fields }),

  /** Optional value (may be omitted). */
  optional: (inner: Validator): Validator => ({ ...inner, optional: true }),

  /** Union of multiple types. */
  union: (...variants: Validator[]): Validator =>
    validator("union", { variants }),

  /** Exact literal value. */
  literal: (value: string | number | boolean): Validator =>
    validator("literal", { value }),

  /** Any valid JSON value. */
  any: (): Validator => validator("any"),
};

// ---------------------------------------------------------------------------
// Runtime validation
// ---------------------------------------------------------------------------

export function validateArgs(
  args: unknown,
  schema: Record<string, Validator>
): { valid: boolean; errors: string[] } {
  const errors: string[] = [];

  if (typeof args !== "object" || args === null) {
    return { valid: false, errors: ["args must be an object"] };
  }

  const obj = args as Record<string, unknown>;

  for (const [key, validator] of Object.entries(schema)) {
    const value = obj[key];

    if (value === undefined || value === null) {
      if (!validator.optional) {
        errors.push(`Missing required field "${key}" (type: ${validator.type})`);
      }
      continue;
    }

    const err = validateValue(value, validator, key);
    if (err) errors.push(err);
  }

  return { valid: errors.length === 0, errors };
}

function validateValue(
  value: unknown,
  validator: Validator,
  path: string
): string | null {
  switch (validator.type) {
    case "string":
      return typeof value === "string"
        ? null
        : `${path}: expected string, got ${typeof value}`;
    case "number":
    case "int":
      return typeof value === "number"
        ? null
        : `${path}: expected number, got ${typeof value}`;
    case "boolean":
      return typeof value === "boolean"
        ? null
        : `${path}: expected boolean, got ${typeof value}`;
    case "id":
      return typeof value === "string"
        ? null
        : `${path}: expected id string, got ${typeof value}`;
    case "null":
      return value === null
        ? null
        : `${path}: expected null, got ${typeof value}`;
    case "any":
      return null;
    case "literal":
      return value === validator.value
        ? null
        : `${path}: expected literal ${JSON.stringify(validator.value)}, got ${JSON.stringify(value)}`;
    case "array":
      if (!Array.isArray(value))
        return `${path}: expected array, got ${typeof value}`;
      if (validator.items) {
        for (let i = 0; i < value.length; i++) {
          const err = validateValue(value[i], validator.items, `${path}[${i}]`);
          if (err) return err;
        }
      }
      return null;
    case "object":
      if (typeof value !== "object" || value === null || Array.isArray(value))
        return `${path}: expected object`;
      if (validator.fields) {
        for (const [k, v] of Object.entries(validator.fields)) {
          const fieldVal = (value as Record<string, unknown>)[k];
          if (fieldVal === undefined && !v.optional) {
            return `${path}.${k}: required field missing`;
          }
          if (fieldVal !== undefined) {
            const err = validateValue(fieldVal, v, `${path}.${k}`);
            if (err) return err;
          }
        }
      }
      return null;
    case "union":
      if (validator.variants) {
        for (const variant of validator.variants) {
          if (validateValue(value, variant, path) === null) return null;
        }
        return `${path}: value does not match any variant`;
      }
      return null;
    default:
      return null;
  }
}
