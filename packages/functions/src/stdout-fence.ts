// Keeps stdout for the host protocol. See `fenceStdout`.

// Bun global used here. Declared locally so apps that type-check this source
// without bun-types still pass `tsc` (same pattern as runtime.ts).
declare const Bun: {
  write(dest: unknown, data: string): unknown;
  stderr: unknown;
};

/**
 * Redirect console.* from user code to stderr so handlers can't accidentally
 * emit a line that looks like a protocol frame and confuse the Rust reader.
 *
 * Before this guard, a handler calling `console.log('{"type":"return",...}')`
 * — either intentionally or by logging an object shaped that way — would be
 * parsed by the host as a real protocol message. Moving all console output
 * to stderr keeps stdout reserved for NDJSON protocol frames only.
 *
 * The original console methods are saved on the console object as
 * `__stdoutLog` etc. in case the runtime itself needs to write diagnostics
 * to stdout for some reason (it currently doesn't).
 *
 * Safe to call more than once. A production server bundle calls it from its
 * entry, before the app modules load, and the runtime's `main()` calls it
 * again.
 */
export function fenceStdout(): void {
  const c = globalThis.console as unknown as Record<string, unknown>;
  if (c.__pylonFenced) return;
  c.__pylonFenced = true;
  const toStderr = (prefix: string) => (...args: unknown[]) => {
    const line = args
      .map((a) => {
        if (typeof a === "string") return a;
        // Error: JSON.stringify yields `{}` because message/stack are
        // non-enumerable. That made `console.error("x:", err)` log as `x: {}`,
        // hiding the real failure from operators. Unwrap by hand.
        if (a instanceof Error) {
          const parts = [a.stack || `${a.name}: ${a.message}`];
          const code = (a as { code?: unknown }).code;
          if (code !== undefined) parts.push(`code=${String(code)}`);
          const cause = (a as { cause?: unknown }).cause;
          if (cause !== undefined) {
            try {
              parts.push(`cause=${cause instanceof Error ? cause.stack || cause.message : JSON.stringify(cause)}`);
            } catch {
              parts.push(`cause=${String(cause)}`);
            }
          }
          return parts.join(" ");
        }
        try {
          return JSON.stringify(a);
        } catch {
          return String(a);
        }
      })
      .join(" ");
    Bun.write(Bun.stderr, `${prefix}${line}\n`);
  };
  // Intentional: we want console.* for user handlers to go to stderr.
  // Overwrite the globals before any user code is loaded.
  c.__stdoutLog = c.log;
  c.log = toStderr("");
  c.info = toStderr("");
  c.warn = toStderr("[warn] ");
  c.error = toStderr("[error] ");
  c.debug = toStderr("[debug] ");
}
