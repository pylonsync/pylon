import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { test } from 'node:test';

function bridge(overrides = {}) {
  const sockets = [];
  class Socket {
    constructor(url, protocols) { this.url = url; this.protocols = protocols; this.bufferedAmount = 0; this.sent = []; sockets.push(this); }
    send(value) { this.sent.push(value); }
    close() { this.closed = true; }
  }
  const context = vm.createContext({
    LibraryManager: { library: {} }, mergeInto: Object.assign,
    TextEncoder, TextDecoder, Uint8Array, AbortController,
    HEAPU8: new Uint8Array(1024), UTF8ToString: x => x,
    WebSocket: Socket, atob: x => Buffer.from(x, 'base64').toString('binary'),
    ...overrides,
  });
  for (const file of ['PylonWeb.jslib', 'PylonWebTransport.jslib'])
    vm.runInContext(readFileSync(new URL(`../../Plugins/WebGL/${file}`, import.meta.url), 'utf8'), context);
  context.PylonWeb = context.LibraryManager.library.$PylonWeb;
  return { api: context.LibraryManager.library, context, sockets };
}

const flush = () => new Promise(resolve => setImmediate(resolve));

test('binary and text frames retain bytes, order, and ownership', () => {
  const { api, context, sockets } = bridge();
  const id = api.PylonWeb_Open('wss://example/shard?v=2', '["ticket.secret"]', 32);
  const socket = sockets[0];
  assert.deepEqual(Array.from(socket.protocols), ['ticket.secret']);
  socket.onopen();
  socket.onmessage({ data: new Uint8Array([1, 0, 255]).buffer });
  socket.onmessage({ data: '✓' });
  assert.equal(api.PylonWeb_Next(id), 3);
  assert.equal(api.PylonWeb_Read(id, 0), 0);
  assert.deepEqual(Array.from(context.HEAPU8.slice(0, 3)), [1, 0, 255]);
  assert.equal(api.PylonWeb_Read(id, 0), 1);
  assert.equal(new TextDecoder().decode(context.HEAPU8.slice(0, 3)), '✓');
  api.PylonWeb_Send(id, 0, 3, 0);
  context.HEAPU8.fill(0);
  assert.equal(new TextDecoder().decode(socket.sent[0]), '✓');
});

test('receive overflow closes; send pressure returns retry without sending', () => {
  const { api, sockets } = bridge();
  const id = api.PylonWeb_Open('ws://example', '[]', 4);
  sockets[0].onopen();
  sockets[0].bufferedAmount = 4;
  assert.equal(api.PylonWeb_Send(id, 0, 1, 0), 0);
  assert.equal(sockets[0].sent.length, 0);
  sockets[0].onmessage({ data: new Uint8Array(5).buffer });
  assert.equal(api.PylonWeb_State(id), 2);
  assert.equal(api.PylonWeb_CloseCode(id), 1009);
  assert.equal(api.PylonWeb_Next(id), -1);
});

test('dispose cancels connecting sockets and repeated cycles release every handle', () => {
  const { api, context, sockets } = bridge();
  for (let i = 0; i < 100; i++) {
    const id = api.PylonWeb_Open('ws://example', '[]', 16);
    api.PylonWeb_Release(id);
    api.PylonWeb_Release(id);
  }
  assert.equal(Object.keys(context.PylonWeb.handles).length, 0);
  assert.ok(sockets.every(s => s.closed && s.onopen === null && s.onmessage === null));
});

test('HTTP keeps error status and response body; dispose aborts pending fetch', async () => {
  let options, resolveFetch;
  const { api, context } = bridge({ fetch: (_, opts) => { options = opts; return new Promise(resolve => { resolveFetch = resolve; }); } });
  const id = api.PylonWeb_Fetch('https://example/api', 'POST', '{"Authorization":"Bearer token"}', 0, 0);
  assert.equal(options.credentials, 'omit');
  assert.equal(options.headers.Authorization, 'Bearer token');
  resolveFetch({ status: 401, arrayBuffer: async () => new TextEncoder().encode('denied').buffer });
  await flush();
  assert.equal(api.PylonWeb_Status(id), 401);
  api.PylonWeb_Read(id, 0);
  assert.equal(new TextDecoder().decode(context.HEAPU8.slice(0, 6)), 'denied');
  api.PylonWeb_Release(id);
  const pending = api.PylonWeb_Fetch('https://example/api', 'GET', '{}', 0, -1);
  api.PylonWeb_Release(pending);
  assert.ok(options.signal.aborted);
  resolveFetch({ status: 200, arrayBuffer: async () => new ArrayBuffer(0) });
  await flush();
  assert.equal(Object.keys(context.PylonWeb.handles).length, 0);
});

test('WebTransport reads partial streams, preserves datagram boundaries, and frees readers', async () => {
  let session;
  class Transport {
    constructor() {
      session = this;
      this.ready = Promise.resolve(); this.closed = new Promise(() => {});
      this.readers = [];
      const readable = () => ({ getReader: () => {
        const r = { read: () => new Promise(resolve => { r.deliver = resolve; }), cancel: async () => { r.cancelled = true; } };
        this.readers.push(r); return r;
      }});
      this.stream = { readable: readable(), writable: { getWriter: () => ({ write: async bytes => { this.sent = bytes; } }) } };
      this.datagrams = { readable: readable(), writable: this.stream.writable, maxDatagramSize: 1000 };
    }
    async createBidirectionalStream() { return this.stream; }
    close() { this.stopped = true; }
  }
  const { api, context } = bridge({ WebTransport: Transport });
  const id = api.PylonWT_Open('https://example/shard', '[]');
  await flush();
  assert.equal(api.PylonWeb_State(id), 1);
  session.readers[0].deliver({ value: new Uint8Array([0, 0, 0, 2, 8, 9]) });
  session.readers[1].deliver({ value: new Uint8Array([2, 4, 6]) });
  await flush();
  assert.equal(api.PylonWT_Read(id, 0, 4, 0), 4);
  assert.equal(api.PylonWT_Read(id, 0, 4, 0), 2);
  assert.deepEqual(Array.from(context.HEAPU8.slice(0, 2)), [8, 9]);
  assert.equal(api.PylonWT_Read(id, 0, 4, 1), 3);
  assert.deepEqual(Array.from(context.HEAPU8.slice(0, 3)), [2, 4, 6]);
  assert.equal(api.PylonWT_MaxDatagram(id), 1000);
  api.PylonWeb_Release(id);
  assert.ok(session.stopped && session.readers.every(r => r.cancelled));
});

test('late WebTransport readiness cannot create a stream after disposal', async () => {
  let ready, created = false;
  class Transport {
    constructor() { this.ready = new Promise(resolve => { ready = resolve; }); this.closed = new Promise(() => {}); }
    createBidirectionalStream() { created = true; return Promise.reject(new Error('must not run')); }
    close() {}
  }
  const { api } = bridge({ WebTransport: Transport });
  const id = api.PylonWT_Open('https://example', '[]');
  api.PylonWeb_Release(id); ready(); await flush();
  assert.equal(created, false);
});

test('WebSocket constructor failure becomes a closed handle and can be released', () => {
  const { api, context } = bridge({ WebSocket: class { constructor() { throw new Error('refused'); } } });
  const id = api.PylonWeb_Open('bad-url', '[]', 16);
  assert.equal(api.PylonWeb_State(id), 2);
  assert.equal(api.PylonWeb_CloseCode(id), 1006);
  api.PylonWeb_Release(id);
  assert.equal(Object.keys(context.PylonWeb.handles).length, 0);
});

test('policy close preserves its code and reason after queued frames', () => {
  const { api, context, sockets } = bridge();
  const id = api.PylonWeb_Open('ws://example', '[]', 100);
  sockets[0].onopen();
  sockets[0].onmessage({ data: 'last' });
  sockets[0].onclose({ code: 1008, reason: 'unauthorized: ticket expired' });
  assert.equal(api.PylonWeb_Next(id), 4);
  api.PylonWeb_Read(id, 0);
  assert.equal(api.PylonWeb_Next(id), -1);
  assert.equal(api.PylonWeb_CloseCode(id), 1008);
  const n = api.PylonWeb_CloseReason(id, 0, 100);
  assert.equal(new TextDecoder().decode(context.HEAPU8.slice(0, n)), 'unauthorized: ticket expired');
});

test('WebTransport open failure permits the connection layer to fall back', async () => {
  class Transport {
    constructor() { this.ready = Promise.reject(new Error('UDP blocked')); this.closed = new Promise(() => {}); }
    close() { this.closedLocally = true; }
  }
  const { api, context } = bridge({ WebTransport: Transport });
  const id = api.PylonWT_Open('https://example/shard', '[]');
  await flush();
  assert.equal(api.PylonWeb_State(id), 2);
  assert.equal(api.PylonWT_Write(id, 0, 1, 0), 0);
  api.PylonWeb_Release(id);
  assert.equal(Object.keys(context.PylonWeb.handles).length, 0);
});

// A fake WebTransport session whose close, reads and writes the test drives.
function fakeTransport() {
  const sessions = [];
  class Transport {
    constructor() {
      sessions.push(this);
      this.ready = Promise.resolve();
      this.closed = new Promise(resolve => { this.closeWith = resolve; });
      this.readers = [];
      this.writes = [];
      const readable = () => ({ getReader: () => {
        const r = { read: () => new Promise(resolve => { r.deliver = resolve; }), cancel: async () => { r.cancelled = true; } };
        this.readers.push(r); return r;
      }});
      const writable = { getWriter: () => ({ write: () => new Promise((resolve, reject) => this.writes.push({ resolve, reject })) }) };
      this.stream = { readable: readable(), writable };
      this.datagrams = { readable: readable(), writable, maxDatagramSize: 1200 };
    }
    async createBidirectionalStream() { return this.stream; }
    close() { this.stopped = true; this.closeWith({ closeCode: 0, reason: '' }); }
  }
  return { Transport, sessions };
}

test('a server close keeps its code and reason when a pending write fails afterwards', async () => {
  const { Transport, sessions } = fakeTransport();
  const { api, context } = bridge({ WebTransport: Transport });
  const id = api.PylonWT_Open('https://example/shard', '[]');
  await flush();
  assert.equal(api.PylonWT_Write(id, 0, 4, 0), 1);
  sessions[0].closeWith({ closeCode: 1, reason: 'unauthorized: no ticket' });
  await flush();
  sessions[0].writes[0].reject(new Error('closed'));
  await flush();
  assert.equal(api.PylonWeb_State(id), 2);
  assert.equal(api.PylonWeb_CloseCode(id), 1);
  const n = api.PylonWeb_CloseReason(id, 0, 256);
  assert.equal(new TextDecoder().decode(context.HEAPU8.slice(0, n)), 'unauthorized: no ticket');
});

test('a local failure keeps its reason when the session then reports closed', async () => {
  const { Transport, sessions } = fakeTransport();
  const { api, context } = bridge({ WebTransport: Transport });
  const id = api.PylonWT_Open('https://example/shard', '[]');
  await flush();
  assert.equal(api.PylonWT_Write(id, 0, 4, 0), 1);
  sessions[0].writes[0].reject(new Error('reset'));
  await flush();
  await flush();
  assert.equal(api.PylonWeb_CloseCode(id), -1);
  const n = api.PylonWeb_CloseReason(id, 0, 256);
  assert.equal(new TextDecoder().decode(context.HEAPU8.slice(0, n)), 'Browser WebTransport write failed');
});

test('a datagram backlog drops the oldest datagrams and keeps the session', async () => {
  const { Transport, sessions } = fakeTransport();
  const { api, context } = bridge({ WebTransport: Transport });
  const id = api.PylonWT_Open('https://example/shard', '[]');
  await flush();
  const datagrams = sessions[0].readers[1];
  for (let i = 0; i < 4100; i++) {
    datagrams.deliver({ value: new Uint8Array([i % 256, i >> 8]) });
    await flush();
  }
  assert.equal(api.PylonWeb_State(id), 1);
  // The first four were dropped; the oldest kept is number 4.
  assert.equal(api.PylonWT_Read(id, 0, 16, 1), 2);
  assert.deepEqual(Array.from(context.HEAPU8.slice(0, 2)), [4, 0]);
});

test('the WebSocket receive bound applies per message; the backlog has its own total', () => {
  const { api, sockets } = bridge();
  const id = api.PylonWeb_Open('ws://example', '[]', 8);
  sockets[0].onopen();
  for (let i = 0; i < 10; i++) sockets[0].onmessage({ data: new Uint8Array(8).buffer });
  assert.equal(api.PylonWeb_State(id), 1);
  sockets[0].onmessage({ data: new Uint8Array(9).buffer });
  assert.equal(api.PylonWeb_State(id), 2);
  assert.equal(api.PylonWeb_CloseCode(id), 1009);
});
