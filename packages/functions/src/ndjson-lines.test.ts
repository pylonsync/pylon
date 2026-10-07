import { expect, test } from "bun:test";
import { createLineSplitter } from "./ndjson-lines";

const enc = new TextEncoder();

function collect(chunks: (string | Uint8Array)[]) {
  const out: string[] = [];
  const s = createLineSplitter((l) => out.push(l));
  for (const c of chunks) s.push(typeof c === "string" ? enc.encode(c) : c);
  s.end();
  return out;
}

test("splits lines inside one chunk and across chunks", () => {
  expect(collect(['{"a":1}\n{"b"', ':2}\n{"c":3}\n'])).toEqual(['{"a":1}', '{"b":2}', '{"c":3}']);
});

test("joins a line spread over many chunks", () => {
  expect(collect(["ab", "cd", "ef", "\n"])).toEqual(["abcdef"]);
});

test("skips blank lines", () => {
  expect(collect(["\n\n  \nx\n\n"])).toEqual(["x"]);
});

test("emits an unterminated last line at end of stream", () => {
  expect(collect(["a\nb"])).toEqual(["a", "b"]);
});

test("decodes a multi-byte character split across chunks", () => {
  const bytes = enc.encode('{"s":"é✓"}\n');
  const out = collect([bytes.slice(0, 7), bytes.slice(7, 9), bytes.slice(9)]);
  expect(JSON.parse(out[0]).s).toBe("é✓");
});

test("a 48MB frame in 64KB chunks parses in linear time", () => {
  const payload = "x".repeat(48 * 1024 * 1024);
  const frame = enc.encode(`{"data":"${payload}"}\n`);
  const lines: string[] = [];
  const s = createLineSplitter((l) => lines.push(l));
  const started = performance.now();
  for (let i = 0; i < frame.length; i += 64 * 1024) s.push(frame.subarray(i, i + 64 * 1024));
  s.end();
  const ms = performance.now() - started;
  expect(lines.length).toBe(1);
  expect(lines[0].length).toBe(frame.length - 1);
  // The old buffer.split reader took minutes on this input.
  expect(ms).toBeLessThan(2000);
});
