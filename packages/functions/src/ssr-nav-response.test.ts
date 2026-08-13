// Regression tests for the client-navigation response shape.
//
// A client-side navigation re-renders the page from __PYLON_DATA__, so the
// markup the server built for it is parsed and thrown away. Measured on a real
// route, that was 16442 bytes of HTML to deliver a 374-byte payload. The nav
// response carries the head metadata and the data, and nothing else.

import { describe, expect, it } from "bun:test";
import { buildHydrationTail, isNavRequest } from "./ssr-runtime";

describe("isNavRequest", () => {
  it("recognizes the header the client sends", () => {
    expect(isNavRequest({ "x-pylon-nav": "1" })).toBe(true);
  });

  it("tolerates surrounding whitespace", () => {
    expect(isNavRequest({ "x-pylon-nav": " 1 " })).toBe(true);
  });

  it("treats a document load as a document load", () => {
    expect(isNavRequest({})).toBe(false);
    expect(isNavRequest(undefined)).toBe(false);
    expect(isNavRequest({ accept: "text/html" })).toBe(false);
  });

  it("refuses any value other than 1", () => {
    // Fail closed: an unrecognized value serves the full page, which is
    // always correct, rather than a payload the caller can't use.
    expect(isNavRequest({ "x-pylon-nav": "0" })).toBe(false);
    expect(isNavRequest({ "x-pylon-nav": "true" })).toBe(false);
    expect(isNavRequest({ "x-pylon-nav": "" })).toBe(false);
  });
});

const TAIL_ARGS = {
  component: "app/dashboard/page",
  layouts: ["app/layout"],
  props: { url: "/dashboard", params: {}, auth: { user_id: "u_1" } },
  ssrData: { "list:[\"Todo\"]": [{ id: "t1" }] },
  manifestRoute: { file: "entry-a1.js", imports: ["chunks/s.js"], css: [] },
  publicPrefix: "/_pylon/build/",
  manifestErr: null,
};

describe("hydration tail, dataOnly", () => {
  it("carries the same payload as a full page tail", () => {
    const data = (html: string) =>
      html.match(/<script id="__PYLON_DATA__"[^>]*>(.*?)<\/script>/s)?.[1];
    expect(data(buildHydrationTail({ ...TAIL_ARGS, dataOnly: true }))).toBe(
      data(buildHydrationTail(TAIL_ARGS))!,
    );
  });

  it("drops the entry script and preloads a navigation doesn't need", () => {
    // The client already has the runtime and resolves the route entry from the
    // build manifest, so these are dead weight on every navigation.
    const nav = buildHydrationTail({ ...TAIL_ARGS, dataOnly: true });
    expect(nav).not.toContain("<script type=\"module\"");
    expect(nav).not.toContain("entry-a1.js");
    const full = buildHydrationTail(TAIL_ARGS);
    expect(full).toContain("entry-a1.js");
  });

  it("still strips the live handles and the request's own headers/cookies", () => {
    // SECURITY: the strip is why this shares buildHydrationTail rather than
    // rebuilding the payload — the session cookie must never reach client JS.
    const nav = buildHydrationTail({
      ...TAIL_ARGS,
      props: {
        ...TAIL_ARGS.props,
        headers: { cookie: "pylon_session=secret" },
        cookies: { pylon_session: "secret" },
        serverData: { list: () => {} },
        response: { setStatus: () => {} },
      },
      dataOnly: true,
    });
    expect(nav).not.toContain("secret");
    expect(nav).not.toContain("serverData");
  });

  it("carries only the binary signed-in bit on a bucketed render", () => {
    // A bucketed render is stored SHARED, so its payload must never carry the
    // rendering user's identity — same rule, data-only or not.
    const nav = buildHydrationTail({
      ...TAIL_ARGS,
      props: { ...TAIL_ARGS.props, auth: { user_id: "u_1", email: "a@b.c" } },
      bucketAuth: { signedIn: true },
      dataOnly: true,
    });
    expect(nav).not.toContain("u_1");
    expect(nav).not.toContain("a@b.c");
    expect(nav).toContain('"signedIn":true');
  });
});
