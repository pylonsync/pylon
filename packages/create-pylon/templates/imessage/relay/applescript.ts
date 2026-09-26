// Send an iMessage through Messages.app with osascript.
//
// The message text and the recipient are passed as ARGUMENTS to a fixed
// AppleScript (`on run argv`), never spliced into script source, so quotes,
// backslashes, newlines, or AppleScript keywords in a message cannot change
// what the script does. osascript is spawned directly, without a shell.

import { isNormalizedHandle } from "../lib/handles";

export const SEND_SCRIPT = `on run argv
  set theHandle to item 2 of argv
  set theText to item 3 of argv
  tell application "Messages"
    set theService to 1st account whose service type = iMessage
    set theBuddy to participant theHandle of theService
    send theText to theBuddy
  end tell
end run`;

/**
 * Fixed first argument. osascript stops option parsing at the first
 * non-option argument, so this keeps a message that starts with "-" from
 * being read as an osascript flag.
 */
export const ARGV_SENTINEL = "pylon-imessage";

export const MAX_SEND_CHARS = 20_000;

export function buildOsascriptArgv(handle: string, text: string): string[] {
  if (!isNormalizedHandle(handle)) throw new Error("Refusing to send: recipient is not a phone number or email");
  if (typeof text !== "string" || text.length === 0) throw new Error("Refusing to send an empty message");
  if (text.length > MAX_SEND_CHARS) throw new Error("Refusing to send: message too long");
  if (text.includes("\u0000")) throw new Error("Refusing to send: message contains a NUL byte");
  return ["osascript", "-e", SEND_SCRIPT, ARGV_SENTINEL, handle, text];
}

export interface SpawnResult {
  exitCode: number;
  stderr: string;
}

export type Spawner = (argv: string[]) => Promise<SpawnResult>;

/** Default spawner: Bun.spawn with an argv array (no shell). */
export const bunSpawner: Spawner = async (argv) => {
  const proc = Bun.spawn(argv, { stdout: "ignore", stderr: "pipe", stdin: "ignore" });
  const timer = setTimeout(() => proc.kill(), 30_000);
  const [exitCode, stderr] = await Promise.all([proc.exited, new Response(proc.stderr).text()]);
  clearTimeout(timer);
  return { exitCode, stderr };
};

export async function sendIMessage(
  handle: string,
  text: string,
  options: { dryRun: boolean; spawn?: Spawner },
): Promise<{ ok: true } | { ok: false; error: string }> {
  let argv: string[];
  try {
    argv = buildOsascriptArgv(handle, text);
  } catch (err) {
    return { ok: false, error: err instanceof Error ? err.message : String(err) };
  }
  if (options.dryRun) return { ok: true };
  const result = await (options.spawn ?? bunSpawner)(argv);
  if (result.exitCode !== 0) {
    return { ok: false, error: `osascript exited ${result.exitCode}: ${result.stderr.trim().slice(0, 300)}` };
  }
  return { ok: true };
}
