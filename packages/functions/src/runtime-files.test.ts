/**
 * Wire shapes of ctx.files.info and ctx.files.read (runtime.ts). Same
 * child-process harness as runtime-vector.test.ts: the probe emits frames,
 * and for read the test replies so the bytes come back decoded.
 */
import { expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const RUNTIME = join(import.meta.dir, "runtime.ts");

const SCRIPT = `
import { buildActionFiles, buildFiles } from ${JSON.stringify(RUNTIME)};

buildFiles("c_i").info("file_1").catch(() => {});
buildActionFiles("c_r").read("file_2").then((f) => {
  process.stdout.write(JSON.stringify({ probe: "read", id: f.id, size: f.size, contentType: f.contentType, text: new TextDecoder().decode(f.data) }) + "\\n");
  process.exit(0);
}).catch((e) => { console.log(JSON.stringify({ probe: "error", message: String(e) })); process.exit(1); });
setTimeout(() => process.exit(2), 3000);
`;

test("info and read emit files_op frames, and read decodes the bytes", async () => {
  const dir = mkdtempSync(join(tmpdir(), "pylon-fn-files-"));
  const scriptPath = join(dir, "probe.ts");
  writeFileSync(scriptPath, SCRIPT);
  const proc = Bun.spawn(["bun", scriptPath], { stdin: "pipe", stdout: "pipe", stderr: "pipe" });
  const frames: Record<string, any>[] = [];
  let replied = false;
  const decoder = new TextDecoder();
  let buffered = "";
  for await (const chunk of proc.stdout) {
    buffered += decoder.decode(chunk);
    const lines = buffered.split("\n");
    buffered = lines.pop() ?? "";
    for (const line of lines) {
      if (!line.trim().startsWith("{")) continue;
      const frame = JSON.parse(line);
      frames.push(frame);
      if (frame.type === "files_op" && frame.op === "read" && !replied) {
        replied = true;
        const data = Buffer.from("hello file").toString("base64");
        proc.stdin.write(
          JSON.stringify({ type: "result", call_id: frame.call_id, data: { id: "file_2", contentType: "text/plain", size: 10, data_base64: data } }) + "\n",
        );
        proc.stdin.flush();
      }
    }
  }
  await proc.exited;
  expect(frames.find((f) => f.type === "files_op" && f.op === "info")).toMatchObject({ call_id: "c_i", file_id: "file_1" });
  expect(frames.find((f) => f.type === "files_op" && f.op === "read")).toMatchObject({ call_id: "c_r", file_id: "file_2" });
  expect(frames.find((f) => f.probe === "read")).toEqual({ probe: "read", id: "file_2", size: 10, contentType: "text/plain", text: "hello file" });
});
