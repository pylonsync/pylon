/**
 * `service({...})` and `buildManifest({ services })`: services reach the
 * manifest as written, and a value the host cannot deploy fails when app.ts
 * runs.
 */
import { expect, test } from "bun:test";
import { buildManifest, entity, field, service } from "./index";

const base = {
  name: "t",
  version: "0",
  entities: [entity("Doc", { title: field.string() })],
  routes: [],
};

const worker = {
  name: "worker",
  dir: "services/worker",
  start: "uvicorn app:app --port 8080",
  urlEnv: "WORKER_URL",
};

test("services are copied into the manifest", () => {
  const full = service({
    ...worker,
    setup: "uv sync --frozen",
    port: 9000,
    healthPath: "/health",
    tokenEnv: "WORKER_TOKEN",
    env: ["MAPS_KEY"],
    vars: { LOG_LEVEL: "info" },
    allowHosts: ["api.example.com", "*.example.org"],
    memoryMb: 4096,
    keepAwake: true,
  });
  expect(buildManifest({ ...base, services: [full] }).services).toEqual([full]);
  expect("services" in buildManifest(base)).toBe(false);
});

test("bad services throw", () => {
  expect(() => service({ ...worker, name: "Worker" })).toThrow("name must start");
  expect(() => service({ ...worker, dir: "../worker" })).toThrow("relative to the app root");
  expect(() => service({ ...worker, dir: "/srv/worker" })).toThrow("relative to the app root");
  expect(() => service({ ...worker, dir: "." })).toThrow("below the app root");
  expect(() => service({ ...worker, start: " " })).toThrow("start must be a non-empty command");
  expect(() => service({ ...worker, setup: "a\rb" })).toThrow("carriage return");
  expect(() => service({ ...worker, port: 80 })).toThrow("port");
  expect(() => service({ ...worker, healthPath: "health" })).toThrow("healthPath");
  expect(() => service({ ...worker, urlEnv: "worker-url" })).toThrow("urlEnv");
  expect(() => service({ ...worker, urlEnv: "PORT" })).toThrow("the host sets it");
  expect(() => service({ ...worker, tokenEnv: "WORKER_URL" })).toThrow("must differ");
  expect(() => service({ ...worker, tokenEnv: "T", env: ["T"] })).toThrow("T is set twice");
  expect(() => service({ ...worker, env: ["A"], vars: { A: "1" } })).toThrow("A is set twice");
  expect(() => service({ ...worker, vars: { A: 1 as never } })).toThrow("vars.A");
  expect(() => service({ ...worker, allowHosts: ["https://example.com"] })).toThrow("allowHosts");
  expect(() => service({ ...worker, allowHosts: ["*"] })).toThrow("allowHosts");
  expect(() => service({ ...worker, memoryMb: 64 })).toThrow("memoryMb");
  expect(() => service({ ...worker, keepAwake: "yes" as never })).toThrow("keepAwake");
});

test("services must not collide", () => {
  expect(() => buildManifest({ ...base, services: [worker, worker] })).toThrow('two services are named "worker"');
  expect(() => buildManifest({ ...base, services: [worker, { ...worker, name: "b" }] })).toThrow("WORKER_URL");
  const nine = Array.from({ length: 9 }, (_, i) => ({ ...worker, name: `s${i}`, urlEnv: `S${i}_URL` }));
  expect(() => buildManifest({ ...base, services: nine })).toThrow("up to 8 services");
});
