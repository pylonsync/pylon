/**
 * Test-side helpers for `pylon test`.
 *
 * The CLI already starts each test FILE with a fresh in-memory database
 * (`PYLON_IN_MEMORY=1`), so tests across files never cross-contaminate.
 * This module covers the finer-grained case: isolating individual
 * `test(...)` blocks within a single file.
 *
 * Two integration patterns are supported:
 *
 * 1. **Manual** — call `resetDb()` from a `beforeEach` hook.
 * 2. **Automatic** — call `installTestIsolation()` at the top of the file
 *    and every `test()` block runs with a reset store.
 *
 * Both require the test file to run under `pylon test` (not raw
 * `bun test`), because resetDb talks to the server via HTTP.
 */

const DEFAULT_BASE_URL = "http://localhost:4321";

/**
 * Reset the in-memory database to empty. Returns when the server confirms.
 *
 * Only works when the server is running in in-memory dev mode — production
 * deployments refuse this call. Safe to no-op when the reset endpoint is
 * unreachable so tests using this helper still work under raw `bun test`
 * (they just won't reset between cases).
 */
export async function resetDb(
  baseUrl: string = DEFAULT_BASE_URL,
): Promise<void> {
  try {
    const res = await fetch(`${baseUrl}/api/__test__/reset`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
    });
    if (!res.ok && res.status !== 404) {
      const body = await res.text().catch(() => "");
      throw new Error(
        `resetDb failed: ${res.status} ${body.slice(0, 200)}`,
      );
    }
  } catch (err: any) {
    // Fetch may throw if the server isn't up yet. Tests that don't need
    // isolation still succeed; tests that do will see pollution and fail
    // loudly on their own assertions. Log so authors can debug.
    // eslint-disable-next-line no-console
    console.warn("[pylon-test] resetDb skipped:", err?.message ?? err);
  }
}

/**
 * Bun-friendly `beforeEach(resetDb)` installer. Looks up Bun's global
 * `beforeEach` via `globalThis`; no-ops under other runners.
 */
export function installTestIsolation(
  baseUrl: string = DEFAULT_BASE_URL,
): void {
  const g = globalThis as any;
  if (typeof g.beforeEach === "function") {
    g.beforeEach(() => resetDb(baseUrl));
  } else {
    // eslint-disable-next-line no-console
    console.warn(
      "[pylon-test] installTestIsolation: no global beforeEach found — run via `pylon test` or `bun test`",
    );
  }
}
