// The `url` page prop carries the request PATH, never the query — the router
// hands over `path_only_owned` and the query arrives separately as
// `search_params`. The name does not say that, and the gap has a cost: a
// login page did `new URL(props.url).searchParams.get("next")`, which
// compiles, looks right, and returns null on every request. It silently
// dropped the pending OIDC authorize request, so signing in to one app in the
// fleet landed on the identity provider's dashboard with no error.
//
// `pathname` is the honest name. These pin that both exist, agree, and reach
// a bucketed page's hydrated tail together — a page that got one but not the
// other would be the same trap in a new place.

import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const runtime = readFileSync(new URL("./ssr-runtime.ts", import.meta.url), "utf8");
const publicTypes = readFileSync(
  new URL("../../react/src/ssr.ts", import.meta.url),
  "utf8",
);

function block(src: string, start: string, end: string): string {
  const a = src.indexOf(start);
  expect(a).toBeGreaterThan(-1);
  const b = src.indexOf(end, a);
  return src.slice(a, b > a ? b : undefined);
}

test("page props carry pathname and url, both from the same value", () => {
  const props = block(runtime, "props = {", "params: msg.params");
  expect(props).toContain("pathname: msg.url");
  expect(props).toContain("url: msg.url");
});

test("a bucketed page's serialized tail carries both", () => {
  const tail = block(runtime, "serializableProps = {", "auth: {");
  expect(tail).toContain("pathname:");
  expect(tail).toContain("url: restProps.url");
});

test("the PPR snapshot carries both", () => {
  const snap = block(runtime, "bucketTailBase = bucketOptIn", "searchParams: jsonClone");
  expect(snap).toContain("pathname: msg.url");
  expect(snap).toContain("url: msg.url");
});

test("PageProps declares pathname and marks url deprecated", () => {
  expect(publicTypes).toContain("pathname: string;");
  const urlDoc = block(publicTypes, "The request path. Same value as `pathname`", "url: string;");
  expect(urlDoc).toContain("@deprecated");
  // The doc has to name the replacement, or it just says "don't" without
  // saying what instead.
  expect(urlDoc).toContain("searchParams");
});

test("the deprecation explains the failure, not just the preference", () => {
  const urlDoc = block(publicTypes, "The request path. Same value as `pathname`", "url: string;");
  expect(urlDoc.toLowerCase()).toContain("query");
  expect(urlDoc).toMatch(/null|silently/);
});
