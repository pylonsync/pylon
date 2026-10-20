// The offline service worker. Every test runs the generated `/pylon-sw.js`
// source (renderServiceWorker), not the TypeScript function, in a fresh VM
// context against a fake ServiceWorkerGlobalScope: what passes here is what
// the browser runs, and a worker body that leaned on module scope would fail.

import { describe, expect, test } from "bun:test";
import { runInNewContext } from "node:vm";
import {
  classifyRequest,
  renderServiceWorker,
  type ServiceWorkerConfig,
} from "./offline-sw";

const ORIGIN = "https://app.example";

const CONFIG: ServiceWorkerConfig = {
  version: "v1",
  buildPrefix: "/_pylon/build/",
  precache: [
    "/_pylon/build/manifest.json",
    "/_pylon/build/client-entry-app__leads__page-abc12345.js",
    "/_pylon/build/chunks/chunk-def67890.js",
    "/_pylon/build/styles-0000abcd.css",
    "/manifest.webmanifest",
    "/icons/192.png",
  ],
  shells: ["/leads", "/map"],
  routes: ["/leads", "/leads/[id]", "/map"],
  startPath: "/leads",
  navigationTimeoutMs: 50,
};

const req = (url: string, mode = "cors", method = "GET") => ({
  url: url.startsWith("http") ? url : ORIGIN + url,
  method,
  mode,
});

describe("classifyRequest", () => {
  test("page loads are navigations", () => {
    expect(classifyRequest(req("/leads", "navigate"), CONFIG, ORIGIN)).toBe("navigate");
    expect(classifyRequest(req("/anything/else", "navigate"), CONFIG, ORIGIN)).toBe("navigate");
  });

  test("build files are cache-first, same-origin or on the CDN", () => {
    expect(classifyRequest(req("/_pylon/build/chunks/a-123.js"), CONFIG, ORIGIN)).toBe("build");
    expect(classifyRequest(req("/_pylon/build/manifest.json"), CONFIG, ORIGIN)).toBe("build");
    const cdn = { ...CONFIG, buildPrefix: "https://assets.example/b1/" };
    expect(classifyRequest(req("https://assets.example/b1/x-123.js"), cdn, ORIGIN)).toBe("build");
    expect(classifyRequest(req("https://assets.example/other.js"), cdn, ORIGIN)).toBe("network");
  });

  test("the web app manifest and icons are served from the cache", () => {
    expect(classifyRequest(req("/manifest.webmanifest"), CONFIG, ORIGIN)).toBe("precached");
    expect(classifyRequest(req("/icons/192.png"), CONFIG, ORIGIN)).toBe("precached");
  });

  test("API, auth, sync, dev and the worker itself are never intercepted", () => {
    for (const path of [
      "/api/entities/Lead",
      "/api/auth/session",
      "/api/sync/pull",
      "/api/fn/listLeads",
      "/auth/callback",
      "/_pylon/dev/live",
      "/events",
      "/studio",
      "/health",
      "/pylon-sw.js",
      "/.well-known/openid-configuration",
    ]) {
      expect(classifyRequest(req(path), CONFIG, ORIGIN)).toBe("network");
      expect(classifyRequest(req(path, "navigate"), CONFIG, ORIGIN)).toBe("network");
    }
  });

  test("writes and other origins are not intercepted", () => {
    expect(classifyRequest(req("/_pylon/build/x.js", "cors", "POST"), CONFIG, ORIGIN)).toBe(
      "network",
    );
    expect(classifyRequest(req("https://maps.example/tile.png"), CONFIG, ORIGIN)).toBe("network");
    expect(classifyRequest(req("/favicon.ico"), CONFIG, ORIGIN)).toBe("network");
  });
});

// ---------------------------------------------------------------------------
// A fake ServiceWorkerGlobalScope.
// ---------------------------------------------------------------------------

type Handler = (event: any) => void;

class FakeCache {
  entries = new Map<string, Response>();
  constructor(private scope: any) {}
  key(r: any): string {
    const url = typeof r === "string" ? r : r.url;
    return new URL(url, ORIGIN).href;
  }
  async match(r: any) {
    const hit = this.entries.get(this.key(r));
    return hit ? hit.clone() : undefined;
  }
  async put(r: any, res: Response) {
    this.entries.set(this.key(r), res);
  }
  async delete(r: any) {
    return this.entries.delete(this.key(r));
  }
  async addAll(urls: string[]) {
    for (const u of urls) {
      const res = await this.scope.fetch(u);
      if (!res.ok) throw new Error(`addAll: ${u} ${res.status}`);
      await this.put(u, res);
    }
  }
}

function makeScope(fetchImpl: (url: string, init?: any) => Promise<Response>) {
  const handlers: Record<string, Handler[]> = {};
  const caches = new Map<string, FakeCache>();
  const fetchLog: Array<{ url: string; headers: Record<string, string> }> = [];
  const s: any = {
    location: { origin: ORIGIN },
    console: { warn: () => {} },
    Response,
    setTimeout,
    clearTimeout,
    claimed: false,
    clients: {
      claim: async () => {
        s.claimed = true;
      },
    },
    caches: {
      open: async (name: string) => {
        if (!caches.has(name)) caches.set(name, new FakeCache(s));
        return caches.get(name)!;
      },
      keys: async () => Array.from(caches.keys()),
      delete: async (name: string) => caches.delete(name),
    },
    fetch: async (input: any, init?: any) => {
      const url = new URL(typeof input === "string" ? input : input.url, ORIGIN).href;
      fetchLog.push({ url, headers: { ...(init?.headers ?? {}) } });
      return fetchImpl(url, init);
    },
    addEventListener: (type: string, h: Handler) => {
      (handlers[type] ||= []).push(h);
    },
  };
  runInNewContext(renderServiceWorker(CONFIG), { self: s, URL });
  async function dispatch(type: string, extra: Record<string, unknown> = {}) {
    let responded: Promise<Response> | null = null;
    const waits: Promise<unknown>[] = [];
    const event = {
      ...extra,
      waitUntil: (p: Promise<unknown>) => waits.push(p),
      respondWith: (p: Promise<Response>) => {
        responded = p;
      },
    };
    for (const h of handlers[type] ?? []) h(event);
    await Promise.all(waits);
    return responded as Promise<Response> | null;
  }
  return { s, caches, fetchLog, dispatch };
}

function html(body: string, headers: Record<string, string> = {}) {
  return new Response(body, {
    status: 200,
    headers: { "Content-Type": "text/html", ...headers },
  });
}

/** A server that answers every URL and marks shell requests with `verdict`. */
function onlineServer(verdict = "1") {
  return async (url: string, init?: any) => {
    const path = new URL(url).pathname;
    if (init?.headers?.["X-Pylon-Offline-Shell"] === "1") {
      return html(`shell ${path}`, { "Pylon-Offline-Shell": verdict });
    }
    return new Response(`file ${path}`, { status: 200 });
  };
}

const offline = async (): Promise<Response> => {
  throw new TypeError("Failed to fetch");
};

const text = async (p: Promise<Response | null>) => (await p)!.text();

describe("install", () => {
  test("precaches the build files and keeps shells the runtime marked", async () => {
    const env = makeScope(onlineServer());
    await env.dispatch("install");
    const assets = env.caches.get("pylon-v1-assets")!;
    for (const url of CONFIG.precache) {
      expect(await assets.match(url)).toBeDefined();
    }
    const shells = env.caches.get("pylon-v1-shells")!;
    expect(await (await shells.match("/leads"))!.text()).toBe("shell /leads");
    expect(await (await shells.match("/map"))!.text()).toBe("shell /map");
    expect(
      env.fetchLog
        .filter((f) => f.headers["X-Pylon-Offline-Shell"] === "1")
        .map((f) => f.url),
    ).toEqual([`${ORIGIN}/leads`, `${ORIGIN}/map`]);
  });

  test("a page the runtime refused is not kept", async () => {
    const env = makeScope(onlineServer("refused; the page read props.auth"));
    await env.dispatch("install");
    const shells = env.caches.get("pylon-v1-shells")!;
    expect(await shells.match("/leads")).toBeUndefined();
  });

  test("a build file that fails to download fails the install", async () => {
    const env = makeScope(async (url) =>
      url.endsWith(".css") ? new Response("", { status: 404 }) : new Response("ok"),
    );
    await expect(env.dispatch("install")).rejects.toThrow();
  });
});

describe("navigation", () => {
  test("online: the network answers", async () => {
    const env = makeScope(onlineServer());
    await env.dispatch("install");
    expect(await text(env.dispatch("fetch", { request: req("/leads", "navigate") }))).toBe(
      "file /leads",
    );
  });

  test("offline: the route's own shell, else the start URL's", async () => {
    let online = true;
    const env = makeScope(async (url, init) => (online ? onlineServer()(url, init) : offline()));
    await env.dispatch("install");
    online = false;
    const nav = (path: string) => text(env.dispatch("fetch", { request: req(path, "navigate") }));
    expect(await nav("/map")).toBe("shell /map");
    expect(await nav("/map/")).toBe("shell /map");
    // A dynamic offline route has no shell of its own.
    expect(await nav("/leads/l_42")).toBe("shell /leads");
    // Not an offline route: the start page beats the browser's offline page.
    expect(await nav("/settings")).toBe("shell /leads");
  });

  test("a slow network on an offline route gives way to the shell", async () => {
    let slow = false;
    const env = makeScope(async (url, init) => {
      if (slow) return new Promise<Response>((r) => setTimeout(() => r(html("late")), 500));
      return onlineServer()(url, init);
    });
    await env.dispatch("install");
    slow = true;
    expect(await text(env.dispatch("fetch", { request: req("/leads", "navigate") }))).toBe(
      "shell /leads",
    );
  });

  test("a slow network on another route is waited for", async () => {
    let slow = false;
    const env = makeScope(async (url, init) => {
      if (slow) return new Promise<Response>((r) => setTimeout(() => r(html("late")), 120));
      return onlineServer()(url, init);
    });
    await env.dispatch("install");
    slow = true;
    expect(await text(env.dispatch("fetch", { request: req("/settings", "navigate") }))).toBe(
      "late",
    );
  });

  test("offline with nothing kept: the network error goes through", async () => {
    const env = makeScope(offline);
    // dispatch() adopts the respondWith promise, so its rejection surfaces here.
    await expect(env.dispatch("fetch", { request: req("/leads", "navigate") })).rejects.toThrow(
      "Failed to fetch",
    );
  });
});

describe("build files and other requests", () => {
  test("cache first; a miss is fetched once and kept", async () => {
    let online = true;
    const env = makeScope(async (url, init) => (online ? onlineServer()(url, init) : offline()));
    await env.dispatch("install");
    online = false;
    expect(
      await text(env.dispatch("fetch", { request: req("/_pylon/build/chunks/chunk-def67890.js") })),
    ).toBe("file /_pylon/build/chunks/chunk-def67890.js");

    online = true;
    const lazy = "/_pylon/build/chunks/lazy-11112222.js";
    await text(env.dispatch("fetch", { request: req(lazy) }));
    online = false;
    expect(await text(env.dispatch("fetch", { request: req(lazy) }))).toBe(`file ${lazy}`);
  });

  test("API and sync requests are left to the network", async () => {
    const env = makeScope(onlineServer());
    for (const path of ["/api/sync/pull", "/api/auth/session", "/api/entities/Lead"]) {
      expect(await env.dispatch("fetch", { request: req(path) })).toBeNull();
    }
    expect(
      await env.dispatch("fetch", { request: req("/api/entities/Lead", "cors", "POST") }),
    ).toBeNull();
  });
});

describe("activate and session changes", () => {
  test("activate deletes other builds' caches, keeps the app's own, and claims", async () => {
    const env = makeScope(onlineServer());
    await env.s.caches.open("pylon-old-assets");
    await env.s.caches.open("pylon-old-shells");
    await env.s.caches.open("app-images");
    await env.dispatch("install");
    await env.dispatch("activate");
    expect((await env.s.caches.keys()).sort()).toEqual([
      "app-images",
      "pylon-v1-assets",
      "pylon-v1-shells",
    ]);
    expect(env.s.claimed).toBe(true);
  });

  test("a change in signed-in state refetches the shells; no change does not", async () => {
    const env = makeScope(onlineServer());
    await env.dispatch("install");
    const shellFetches = () =>
      env.fetchLog.filter((f) => f.headers["X-Pylon-Offline-Shell"] === "1").length;
    expect(shellFetches()).toBe(2);
    await env.dispatch("message", { data: { type: "pylon:session", signedIn: true } });
    expect(shellFetches()).toBe(4);
    await env.dispatch("message", { data: { type: "pylon:session", signedIn: true } });
    expect(shellFetches()).toBe(4);
    await env.dispatch("message", { data: { type: "pylon:session", signedIn: false } });
    expect(shellFetches()).toBe(6);
  });

  test("signing out drops a shell the server now redirects", async () => {
    let signedIn = true;
    const env = makeScope(async (url, init) => {
      if (!signedIn && init?.headers?.["X-Pylon-Offline-Shell"] === "1") {
        const r = html("sign in");
        Object.defineProperty(r, "redirected", { value: true });
        return r;
      }
      return onlineServer()(url, init);
    });
    await env.dispatch("install");
    signedIn = false;
    await env.dispatch("message", { data: { type: "pylon:session", signedIn: false } });
    expect(await env.caches.get("pylon-v1-shells")!.match("/leads")).toBeUndefined();
  });
});
