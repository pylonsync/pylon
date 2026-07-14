import { expect, test } from "bun:test";

import { action, mutation, query } from "./define";
import type { FnDefinition } from "./types";
import { v } from "./validators";

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Expect<T extends true> = T;
type ArgsOf<TDefinition> =
  TDefinition extends FnDefinition<infer TArgs, unknown> ? TArgs : never;
type IsAny<T> = 0 extends 1 & T ? true : false;

const inferredMutation = mutation({
  args: {
    required: v.string(),
    optional: v.optional(v.number()),
    list: v.array(v.int()),
    nested: v.object({
      enabled: v.boolean(),
      label: v.optional(v.string()),
    }),
    union: v.union(v.literal("open"), v.literal(404), v.null()),
    literal: v.literal("fixed"),
    anything: v.any(),
  },
  async handler(_ctx, args) {
    const required: string = args.required;
    const optional: number | undefined = args.optional;
    const list: number[] = args.list;
    const nested: { enabled: boolean; label?: string } = args.nested;
    const union: "open" | 404 | null = args.union;
    const literal: "fixed" = args.literal;
    const anything: any = args.anything;

    return { required, optional, list, nested, union, literal, anything };
  },
});

type InferredArgs = ArgsOf<typeof inferredMutation>;
type _RequiredValue = Expect<Equal<InferredArgs["required"], string>>;
type _RequiredKey = Expect<
  Equal<{} extends Pick<InferredArgs, "required"> ? true : false, false>
>;
type _OptionalValue = Expect<
  Equal<InferredArgs["optional"], number | undefined>
>;
type _OptionalKey = Expect<
  Equal<{} extends Pick<InferredArgs, "optional"> ? true : false, true>
>;
type _Array = Expect<Equal<InferredArgs["list"], number[]>>;
type _ObjectRequired = Expect<
  Equal<InferredArgs["nested"]["enabled"], boolean>
>;
type _ObjectOptional = Expect<
  Equal<InferredArgs["nested"]["label"], string | undefined>
>;
type _Union = Expect<Equal<InferredArgs["union"], "open" | 404 | null>>;
type _Literal = Expect<Equal<InferredArgs["literal"], "fixed">>;
type _Any = Expect<IsAny<InferredArgs["anything"]>>;

const publicQuery = query({
  auth: "public",
  args: { id: v.id("Item") },
  async handler(ctx, args) {
    const userId: string | null = ctx.auth.userId;
    const id: string = args.id;
    return { id, userId };
  },
});

const guestAction = action({
  auth: "guest",
  args: { mode: v.union(v.literal("fast"), v.literal("safe")) },
  async handler(ctx, args) {
    const userId: string | null = ctx.auth.userId;
    const mode: "fast" | "safe" = args.mode;
    return { mode, userId };
  },
});

type ExplicitArgs = { name: string; note?: string };
type ExplicitReturn = { ok: boolean };

const explicitMutation = mutation<ExplicitArgs, ExplicitReturn>({
  args: {
    name: v.string(),
    note: v.optional(v.string()),
  },
  async handler(_ctx, args) {
    const explicit: ExplicitArgs = args;
    return { ok: explicit.name.length > 0 };
  },
});

type _ExplicitArgs = Expect<
  Equal<ArgsOf<typeof explicitMutation>, ExplicitArgs>
>;

test("validator schemas remain available at runtime", () => {
  expect(inferredMutation.args?.required.type).toBe("string");
  expect(inferredMutation.args?.optional.optional).toBe(true);
  expect(publicQuery.auth).toBe("public");
  expect(guestAction.auth).toBe("guest");
  expect(explicitMutation.type).toBe("mutation");
});
