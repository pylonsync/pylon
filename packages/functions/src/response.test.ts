import { describe, expect, test } from "bun:test";
import { response } from "./response";

describe("ctx.response", () => {
  test("defaults to an empty 200", () => {
    expect(response()).toEqual({ __pylonResponse: 1, status: 200, headers: {}, body: "" });
  });

  test("contentType sets the content-type header and replaces one in headers", () => {
    const r = response({
      contentType: "text/xml",
      headers: { "Content-Type": "text/plain", "X-Id": "1" },
      body: "<Response/>",
    });
    expect(r.headers).toEqual({ "X-Id": "1", "content-type": "text/xml" });
    expect(r.body).toBe("<Response/>");
  });

  test("rejects out-of-range and non-integer status", () => {
    for (const status of [99, 101, 600, 200.5, Number.NaN]) {
      expect(() => response({ status })).toThrow("status");
    }
  });

  test("rejects a body on 204", () => {
    expect(() => response({ status: 204, body: "x" })).toThrow("cannot carry a body");
    expect(response({ status: 204 }).status).toBe(204);
  });

  test("rejects header injection and invalid names", () => {
    expect(() => response({ headers: { "X-A": "a\r\nSet-Cookie: x=1" } })).toThrow("control");
    expect(() => response({ headers: { "X-A": "café" } })).toThrow("control");
    expect(() => response({ headers: { "X A": "v" } })).toThrow("token");
    expect(() => response({ contentType: "text/xml\r\nX: y" })).toThrow("control");
  });

  test("rejects headers the server owns, in any case", () => {
    for (const name of ["Content-Length", "set-cookie", "Access-Control-Allow-Origin", "Connection"]) {
      expect(() => response({ headers: { [name]: "v" } })).toThrow("set by the server");
    }
  });
});
