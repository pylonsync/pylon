import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { waitForPackages } from './npm-postflight.mjs';

async function registry(t, handler) {
  const server = createServer(handler);
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => { server.closeAllConnections(); server.close(); });
  return `http://127.0.0.1:${server.address().port}`;
}
const options = { intervalMs: 5, timeoutMs: 200, requestTimeoutMs: 2000, log() {} };

test('checks packages concurrently and retries missing tarballs', async t => {
  const pending = [];
  const queries = new Set();
  let heads = 0;
  const base = await registry(t, (req, res) => {
    if (req.method === 'HEAD') {
      res.writeHead(++heads <= 2 ? 404 : 200).end();
      return;
    }
    queries.add(req.url);
    const name = decodeURIComponent(new URL(req.url, base).pathname.split('/')[1]);
    pending.push(() => res.end(JSON.stringify({ name, version: '1.0.0', dist: { tarball: `${base}/archive` } })));
    // Neither package can finish until both have started.
    if (pending.length === 2) pending.splice(0).forEach(reply => reply());
  });
  await waitForPackages([{ name: '@test/a', version: '1.0.0' }, { name: '@test/b', version: '1.0.0' }], { ...options, registry: base, timeoutMs: 2000 });
  assert.ok(heads >= 4);
  assert.ok(queries.size >= 4);
});

test('rejects a successful response for the wrong version', async t => {
  const base = await registry(t, (_req, res) => res.end(JSON.stringify({ name: 'a', version: '0.9.0', dist: { tarball: 'unused' } })));
  await assert.rejects(waitForPackages([{ name: 'a', version: '1.0.0' }], { ...options, registry: base }), /does not match/);
});

test('reports all missing packages after the common deadline', async t => {
  const base = await registry(t, (_req, res) => res.writeHead(404).end());
  await assert.rejects(waitForPackages([{ name: 'a', version: '1' }, { name: 'b', version: '1' }], { ...options, registry: base }), error => {
    assert.match(error.message, /a@1:/);
    assert.match(error.message, /b@1:/);
    return true;
  });
});

test('bounds a stalled registry request', async t => {
  const base = await registry(t, () => {});
  await assert.rejects(waitForPackages([{ name: 'a', version: '1' }], { ...options, registry: base }), /Publication check failed/);
});
