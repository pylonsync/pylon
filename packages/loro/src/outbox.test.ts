// CrdtOutbox against real LoroDocs. A fake server holds its own doc, merges
// what clients send, and relays each merge to the other clients, the way
// POST /api/crdt and the binary broadcast do.
import { describe, expect, test } from "bun:test";
import { LoroDoc, LoroMap } from "loro-crdt";
import { Schema } from "prosemirror-model";
import { schema as basic } from "prosemirror-schema-basic";
import { CrdtOutbox, isRetryable, type OutboxSend } from "./outbox";
import { readProseMirror, writeProseMirror } from "./prosemirror/index";

const tick = () => new Promise((r) => setTimeout(r, 0));

async function settle(): Promise<void> {
  for (let i = 0; i < 20; i++) await tick();
}

function httpError(status: number): Error & { status: number } {
  return Object.assign(new Error(`status ${status}`), { status });
}

class FakeServer {
  readonly doc = new LoroDoc();
  readonly received: Uint8Array[] = [];
  private readonly clients = new Set<LoroDoc>();

  connect(client: LoroDoc): void {
    client.import(this.doc.export({ mode: "snapshot" }));
    this.clients.add(client);
  }

  /** A send that merges into the server doc and relays to every client. */
  sender(): OutboxSend {
    return async (update) => {
      this.received.push(update);
      this.doc.import(update);
      const merged = this.doc.export({ mode: "snapshot" });
      for (const c of this.clients) c.import(merged);
    };
  }
}

function text(doc: LoroDoc): string {
  return doc.getText("t").toString();
}

describe("CrdtOutbox", () => {
  test("a local edit reaches the server and the other client", async () => {
    const server = new FakeServer();
    const a = new LoroDoc();
    const b = new LoroDoc();
    server.connect(a);
    server.connect(b);
    const outbox = new CrdtOutbox(a, server.sender());
    a.getText("t").insert(0, "hello");
    a.commit();
    await settle();
    expect(text(server.doc)).toBe("hello");
    expect(text(b)).toBe("hello");
    expect(outbox.state).toBe("idle");
    expect(outbox.hasUnsent()).toBe(false);
  });

  test("two clients editing at once converge", async () => {
    const server = new FakeServer();
    const a = new LoroDoc();
    const b = new LoroDoc();
    server.connect(a);
    server.connect(b);
    new CrdtOutbox(a, server.sender());
    new CrdtOutbox(b, server.sender());
    a.getText("t").insert(0, "left ");
    a.commit();
    b.getText("t").insert(0, "right");
    b.commit();
    await settle();
    expect(text(a)).toBe(text(b));
    expect(text(server.doc)).toBe(text(a));
    expect(text(a)).toContain("left");
    expect(text(a)).toContain("right");
  });

  test("commits made during a request go out together in the next one", async () => {
    const server = new FakeServer();
    const a = new LoroDoc();
    server.connect(a);
    let release: () => void = () => {};
    let calls = 0;
    const send: OutboxSend = async (update, opts) => {
      calls += 1;
      if (calls === 1) await new Promise<void>((r) => (release = r));
      await server.sender()(update, opts);
    };
    const outbox = new CrdtOutbox(a, send);
    a.getText("t").insert(0, "a");
    a.commit();
    await tick();
    expect(outbox.state).toBe("sending");
    for (const ch of ["b", "c", "d"]) {
      a.getText("t").insert(a.getText("t").length, ch);
      a.commit();
    }
    await tick();
    expect(calls).toBe(1);
    release();
    await settle();
    expect(calls).toBe(2);
    expect(text(server.doc)).toBe("abcd");
  });

  test("a failed request is sent again, with the edits made since", async () => {
    const server = new FakeServer();
    const a = new LoroDoc();
    server.connect(a);
    const timers: Array<() => void> = [];
    let fail = true;
    const send: OutboxSend = async (update, opts) => {
      if (fail) throw new Error("network down");
      await server.sender()(update, opts);
    };
    const outbox = new CrdtOutbox(a, send, {
      setTimer: (fn) => timers.push(fn),
      clearTimer: () => {},
    });
    a.getText("t").insert(0, "one ");
    a.commit();
    await settle();
    expect(outbox.state).toBe("retrying");
    expect(text(server.doc)).toBe("");
    a.getText("t").insert(4, "two");
    a.commit();
    await settle();
    expect(timers.length).toBe(1); // no second request while waiting
    fail = false;
    timers.shift()!();
    await settle();
    expect(text(server.doc)).toBe("one two");
    expect(outbox.state).toBe("idle");
  });

  test("retry delays double up to the limit", async () => {
    const a = new LoroDoc();
    const delays: number[] = [];
    const timers: Array<() => void> = [];
    const outbox = new CrdtOutbox(a, async () => Promise.reject(httpError(503)), {
      retryBaseMs: 100,
      retryMaxMs: 300,
      setTimer: (fn, ms) => {
        delays.push(ms);
        timers.push(fn);
      },
      clearTimer: () => {},
    });
    a.getText("t").insert(0, "x");
    a.commit();
    for (let i = 0; i < 4; i++) {
      await settle();
      timers.shift()!();
    }
    await settle();
    expect(delays).toEqual([100, 200, 300, 300, 300]);
    expect(outbox.state).toBe("retrying");
  });

  test("retryNow skips the wait", async () => {
    const server = new FakeServer();
    const a = new LoroDoc();
    server.connect(a);
    let fail = true;
    const outbox = new CrdtOutbox(
      a,
      async (u, o) => {
        if (fail) throw new Error("offline");
        await server.sender()(u, o);
      },
      { setTimer: () => "handle", clearTimer: () => {} },
    );
    a.getText("t").insert(0, "x");
    a.commit();
    await settle();
    fail = false;
    outbox.retryNow();
    await settle();
    expect(text(server.doc)).toBe("x");
  });

  test("a refused edit stops the outbox", async () => {
    const a = new LoroDoc();
    let calls = 0;
    const outbox = new CrdtOutbox(a, async () => {
      calls += 1;
      throw httpError(403);
    });
    a.getText("t").insert(0, "x");
    a.commit();
    await settle();
    expect(outbox.state).toBe("rejected");
    expect((outbox.error as { status: number }).status).toBe(403);
    a.getText("t").insert(1, "y");
    a.commit();
    await settle();
    expect(calls).toBe(1);
    expect(outbox.hasUnsent()).toBe(true);
  });

  test("edits that came from the server are not sent back", async () => {
    const server = new FakeServer();
    const a = new LoroDoc();
    const b = new LoroDoc();
    server.connect(a);
    server.connect(b);
    new CrdtOutbox(b, server.sender());
    // A large edit from b reaches a through the server.
    b.getText("t").insert(0, "x".repeat(5000));
    b.commit();
    await settle();
    const sends: Uint8Array[] = [];
    const send = server.sender();
    new CrdtOutbox(a, async (u, o) => {
      sends.push(u);
      await send(u, o);
    });
    a.getText("t").insert(0, "!");
    a.commit();
    await settle();
    expect(sends.length).toBe(1);
    expect(sends[0].length).toBeLessThan(200);
    expect(text(b).startsWith("!")).toBe(true);
  });

  test("edits made before the outbox existed are sent", async () => {
    const server = new FakeServer();
    const a = new LoroDoc();
    server.connect(a);
    a.getText("t").insert(0, "early");
    a.commit();
    new CrdtOutbox(a, server.sender());
    await settle();
    expect(text(server.doc)).toBe("early");
  });

  test("flushOnHide sends with keepalive while a request is in flight", async () => {
    const a = new LoroDoc();
    const seen: boolean[] = [];
    let release: () => void = () => {};
    const outbox = new CrdtOutbox(a, async (_u, { keepalive }) => {
      seen.push(keepalive);
      if (!keepalive) await new Promise<void>((r) => (release = r));
    });
    a.getText("t").insert(0, "x");
    a.commit();
    await tick();
    outbox.flushOnHide();
    await settle();
    expect(seen).toEqual([false, true]);
    expect(outbox.hasUnsent()).toBe(false);
    release();
    await settle();
  });

  test("dispose stops sending", async () => {
    const a = new LoroDoc();
    let calls = 0;
    const outbox = new CrdtOutbox(a, async () => {
      calls += 1;
    });
    outbox.dispose();
    a.getText("t").insert(0, "x");
    a.commit();
    await settle();
    expect(calls).toBe(0);
  });

  test("a ProseMirror document edited in two copies converges", async () => {
    const schema = new Schema({ nodes: basic.spec.nodes, marks: basic.spec.marks });
    const server = new FakeServer();
    // The server creates the field's container on insert.
    server.doc.getMap("row").setContainer("body", new LoroMap());
    server.doc.commit();
    const a = new LoroDoc();
    const b = new LoroDoc();
    server.connect(a);
    server.connect(b);
    new CrdtOutbox(a, server.sender());
    new CrdtOutbox(b, server.sender());
    const para = (s: string) => schema.node("paragraph", null, [schema.text(s)]);
    writeProseMirror(a, "body", schema, schema.node("doc", null, [para("from a")]));
    await settle();
    const current = readProseMirror(b, "body", schema)!;
    writeProseMirror(b, "body", schema, current.copy(current.content.append(schema.node("doc", null, [para("from b")]).content)));
    await settle();
    const expected = ["from a", "from b"];
    for (const d of [a, b, server.doc]) {
      const node = readProseMirror(d, "body", schema)!;
      const paragraphs: string[] = [];
      node.forEach((n) => paragraphs.push(n.textContent));
      expect(paragraphs).toEqual(expected);
    }
  });
});

describe("isRetryable", () => {
  test("network errors, 5xx, 408 and 429 retry; other 4xx do not", () => {
    expect(isRetryable(new Error("x"))).toBe(true);
    expect(isRetryable(httpError(500))).toBe(true);
    expect(isRetryable(httpError(503))).toBe(true);
    expect(isRetryable(httpError(408))).toBe(true);
    expect(isRetryable(httpError(429))).toBe(true);
    expect(isRetryable(httpError(400))).toBe(false);
    expect(isRetryable(httpError(403))).toBe(false);
    expect(isRetryable(httpError(404))).toBe(false);
  });
});
