// The MCP protocol surface, tested through `dispatch` — the same entry point
// the HTTP route calls. What matters here is the wire contract: a client that
// speaks MCP must be able to complete a handshake, list tools, and call one,
// and must get a well-formed JSON-RPC error when it asks for something else.

import { describe, expect, test } from "bun:test";
import {
  INVALID_PARAMS,
  INVALID_REQUEST,
  LATEST_PROTOCOL_VERSION,
  METHOD_NOT_FOUND,
  SUPPORTED_PROTOCOL_VERSIONS,
  TOOLS,
  dispatch,
  isSupportedProtocolVersion,
  mcpManifest,
} from "./mcp";

const rpc = (method: string, params?: Record<string, unknown>, id: string | number = 1) => ({
  jsonrpc: "2.0" as const,
  id,
  method,
  ...(params ? { params } : {}),
});

describe("handshake", () => {
  test("initialize answers in the client's version when we speak it", async () => {
    const res = await dispatch(rpc("initialize", { protocolVersion: "2025-06-18" }));
    expect(res?.error).toBeUndefined();
    const result = res?.result as any;
    expect(result.protocolVersion).toBe("2025-06-18");
    expect(result.serverInfo.name).toBe("pylonsync");
    expect(result.capabilities.tools).toBeDefined();
    // Instructions tell the model WHEN to use the server, not what it is.
    expect(result.instructions).toContain("Use these tools when");
  });

  test("initialize falls back to our newest version for an unknown one", async () => {
    const res = await dispatch(rpc("initialize", { protocolVersion: "1999-01-01" }));
    expect((res?.result as any).protocolVersion).toBe(LATEST_PROTOCOL_VERSION);
  });

  test("server/discover needs no handshake and lists every version", async () => {
    const res = await dispatch(rpc("server/discover"));
    const result = res?.result as any;
    expect(result.protocolVersions).toEqual([...SUPPORTED_PROTOCOL_VERSIONS]);
    expect(result.serverInfo.name).toBe("pylonsync");
  });

  test("a notification gets no response at all", async () => {
    // No `id` → the transport answers 202 with no body.
    expect(await dispatch({ jsonrpc: "2.0", method: "notifications/initialized" })).toBeNull();
  });

  test("ping answers empty", async () => {
    const res = await dispatch(rpc("ping"));
    expect(res?.result).toEqual({});
  });
});

describe("tools", () => {
  test("every tool has a name, a description, and an object schema", () => {
    expect(TOOLS.length).toBeGreaterThan(0);
    for (const tool of TOOLS) {
      expect(tool.name).toMatch(/^[a-z][a-z0-9_]*$/);
      // The description is the whole basis on which a model picks a tool.
      expect(tool.description.length).toBeGreaterThan(80);
      expect((tool.inputSchema as any).type).toBe("object");
      const props = (tool.inputSchema as any).properties ?? {};
      for (const [key, schema] of Object.entries<any>(props)) {
        expect(schema.type, `${tool.name}.${key} needs a type`).toBeDefined();
        expect(schema.description, `${tool.name}.${key} needs a description`).toBeTruthy();
      }
    }
  });

  test("tools/list returns them", async () => {
    const res = await dispatch(rpc("tools/list"));
    const names = (res?.result as any).tools.map((t: any) => t.name);
    expect(names).toContain("search_pylon_docs");
    expect(names).toContain("read_pylon_doc");
    expect(names).toContain("list_pylon_templates");
    expect(names).toContain("get_pylon_skill");
  });

  test("list_pylon_templates returns real scaffold commands", async () => {
    const res = await dispatch(rpc("tools/call", { name: "list_pylon_templates", arguments: {} }));
    const result = res?.result as any;
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain("npm create @pylonsync/pylon@latest");
    expect(result.structuredContent.templates.length).toBeGreaterThan(3);
    for (const t of result.structuredContent.templates) {
      expect(t.command).toContain(`--template ${t.template}`);
      expect(t.source).toStartWith("https://github.com/pylonsync/pylon");
    }
  });

  test("list_pylon_templates filters on the query", async () => {
    const res = await dispatch(
      rpc("tools/call", { name: "list_pylon_templates", arguments: { query: "zzzz-no-such-thing" } }),
    );
    expect((res?.result as any).content[0].text).toBe("No matching templates.");
  });

  test("a tool called with bad arguments reports a tool error, not a protocol error", async () => {
    // MCP distinguishes the two: a protocol error means the CLIENT is broken,
    // a tool error means the call failed and the model should read why.
    const res = await dispatch(rpc("tools/call", { name: "search_pylon_docs", arguments: {} }));
    expect(res?.error).toBeUndefined();
    expect((res?.result as any).isError).toBe(true);
  });

  test("an unknown tool is a protocol error", async () => {
    const res = await dispatch(rpc("tools/call", { name: "drop_database", arguments: {} }));
    expect(res?.error?.code).toBe(INVALID_PARAMS);
  });
});

describe("errors", () => {
  test("an unknown method comes back as method-not-found", async () => {
    const res = await dispatch(rpc("sampling/createMessage"));
    expect(res?.error?.code).toBe(METHOD_NOT_FOUND);
  });

  test("unimplemented capabilities say so instead of returning empty lists", async () => {
    // An empty `resources/list` reads as "there are none right now"; a client
    // then keeps asking. METHOD_NOT_FOUND says "never".
    for (const method of ["resources/list", "prompts/list"]) {
      expect((await dispatch(rpc(method)))?.error?.code).toBe(METHOD_NOT_FOUND);
    }
  });

  test("a non-object message is an invalid request", async () => {
    expect((await dispatch("hello"))?.error?.code).toBe(INVALID_REQUEST);
    expect((await dispatch([]))?.error?.code).toBe(INVALID_REQUEST);
  });

  test("a request missing jsonrpc is rejected but still answered", async () => {
    const res = await dispatch({ id: 7, method: "tools/list" });
    expect(res?.id).toBe(7);
    expect(res?.error?.code).toBe(INVALID_REQUEST);
  });
});

describe("protocol versions", () => {
  test("the supported list is newest first and every entry is a date", () => {
    expect(SUPPORTED_PROTOCOL_VERSIONS[0]).toBe(LATEST_PROTOCOL_VERSION);
    const sorted = [...SUPPORTED_PROTOCOL_VERSIONS].sort().reverse();
    expect([...SUPPORTED_PROTOCOL_VERSIONS]).toEqual(sorted);
    for (const v of SUPPORTED_PROTOCOL_VERSIONS) expect(v).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  test("version support is an exact match", () => {
    expect(isSupportedProtocolVersion("2025-06-18")).toBe(true);
    expect(isSupportedProtocolVersion("2019-01-01")).toBe(false);
    expect(isSupportedProtocolVersion("")).toBe(false);
  });
});

describe("manifest", () => {
  test("names the transport, the URL, and every tool", () => {
    const m = mcpManifest();
    expect(m.remotes[0]).toEqual({
      type: "streamable-http",
      url: "https://www.pylonsync.com/mcp",
    });
    expect(m.authentication.type).toBe("none");
    expect(m.tools.map((t) => t.name).sort()).toEqual(TOOLS.map((t) => t.name).sort());
  });
});
