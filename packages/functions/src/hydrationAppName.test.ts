// Regression: the browser client namespaces its storage by the app's manifest
// name, but the name only lives server-side. The SSR runtime surfaces it as
// PYLON_APP_NAME and buildHydrationTail forwards it into __PYLON_DATA__ as `app`
// so the client can `configureClient({ appName })` at hydrate — before any token
// read. These pin that the field appears exactly when the env is set.

import { expect, test } from "bun:test";

import { buildHydrationTail } from "./ssr-runtime";

const base = {
  component: "app/page.tsx",
  layouts: [] as string[],
  props: {},
  ssrData: {},
  manifestRoute: null,
  publicPrefix: "/_pylon/build/",
  manifestErr: null,
  dataOnly: true, // return just the __PYLON_DATA__ script
};

function withEnv(value: string | undefined, fn: () => void) {
  const prev = process.env.PYLON_APP_NAME;
  if (value === undefined) delete process.env.PYLON_APP_NAME;
  else process.env.PYLON_APP_NAME = value;
  try {
    fn();
  } finally {
    if (prev === undefined) delete process.env.PYLON_APP_NAME;
    else process.env.PYLON_APP_NAME = prev;
  }
}

test("injects app from PYLON_APP_NAME", () => {
  withEnv("revtrail", () => {
    expect(buildHydrationTail(base)).toContain('"app":"revtrail"');
  });
});

test("omits app when PYLON_APP_NAME is unset", () => {
  withEnv(undefined, () => {
    expect(buildHydrationTail(base)).not.toContain('"app"');
  });
});
