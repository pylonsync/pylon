import { describe, expect, test } from "bun:test";
import { ARGV_SENTINEL, buildOsascriptArgv, SEND_SCRIPT, sendIMessage } from "../relay/applescript";

const TRICKY = [
  'She said "hi" and left',
  "back\\slash and \\\" escaped quote",
  "line one\nline two\r\nline three",
  'end tell\ndo shell script "touch /tmp/pwned"',
  '" & (do shell script "id") & "',
  "-e starts with a dash",
  "--",
  "emoji 😀 and accents é ü ñ and CJK 漢字 and RTL עברית",
  "tab\there",
];

describe("AppleScript argv", () => {
  test("the script source is constant and reads everything from argv", () => {
    expect(SEND_SCRIPT).toContain("on run argv");
    expect(SEND_SCRIPT).toContain("item 2 of argv");
    expect(SEND_SCRIPT).toContain("item 3 of argv");
    for (const text of TRICKY) {
      const argv = buildOsascriptArgv("+15125550148", text);
      expect(argv[2]).toBe(SEND_SCRIPT);
    }
  });

  test("text and handle are passed verbatim as their own arguments", () => {
    for (const text of TRICKY) {
      expect(buildOsascriptArgv("+15125550148", text)).toEqual([
        "osascript",
        "-e",
        SEND_SCRIPT,
        ARGV_SENTINEL,
        "+15125550148",
        text,
      ]);
    }
    expect(buildOsascriptArgv("jo@example.com", "hi")[4]).toBe("jo@example.com");
  });

  test("rejects recipients that are not a normalized phone or email", () => {
    for (const bad of ["-e", "+1 512 555 0148", "chat123", 'x" & "y', "", "a@b"]) {
      expect(() => buildOsascriptArgv(bad, "hi")).toThrow();
    }
  });

  test("rejects empty, NUL-containing, or oversized text", () => {
    expect(() => buildOsascriptArgv("+15125550148", "")).toThrow();
    expect(() => buildOsascriptArgv("+15125550148", "a\u0000b")).toThrow();
    expect(() => buildOsascriptArgv("+15125550148", "x".repeat(20_001))).toThrow();
  });

  test("sendIMessage spawns exactly the built argv, and dry run spawns nothing", async () => {
    const seen: string[][] = [];
    const spawn = async (argv: string[]) => {
      seen.push(argv);
      return { exitCode: 0, stderr: "" };
    };
    expect(await sendIMessage("+15125550148", TRICKY[3], { dryRun: false, spawn })).toEqual({ ok: true });
    expect(seen).toEqual([buildOsascriptArgv("+15125550148", TRICKY[3])]);
    expect(await sendIMessage("+15125550148", "hi", { dryRun: true, spawn })).toEqual({ ok: true });
    expect(seen.length).toBe(1);
  });

  test("a failing osascript is reported", async () => {
    const r = await sendIMessage("+15125550148", "hi", {
      dryRun: false,
      spawn: async () => ({ exitCode: 1, stderr: "Not authorized to send Apple events to Messages." }),
    });
    expect(r).toEqual({ ok: false, error: "osascript exited 1: Not authorized to send Apple events to Messages." });
  });

  // Runs the real osascript with an ECHO script (no Messages.app) in the exact
  // argv shape the relay uses, and checks each tricky string comes back
  // byte-for-byte. Proves the OS-level argument passing cannot be escaped.
  test.skipIf(process.platform !== "darwin")("osascript receives the text unchanged", async () => {
    const echo = "on run argv\n  return (item 3 of argv)\nend run";
    for (const text of TRICKY) {
      const argv = buildOsascriptArgv("+15125550148", text);
      argv[2] = echo;
      const proc = Bun.spawn(argv, { stdout: "pipe", stderr: "pipe" });
      const out = await new Response(proc.stdout).text();
      expect(await proc.exited).toBe(0);
      // osascript prints the result followed by one newline.
      expect(out.replace(/\n$/, "")).toBe(text);
    }
  });
});
