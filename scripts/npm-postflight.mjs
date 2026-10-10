import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';

// Check each version independently so propagation delays do not add together.
export async function waitForPackages(packages, {
  registry = 'https://registry.npmjs.org',
  timeoutMs = 600_000,
  intervalMs = 15_000,
  requestTimeoutMs = 10_000,
  log = console.log,
} = {}) {
  const deadline = Date.now() + timeoutMs;
  async function request(url, method = 'GET') {
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error('deadline reached');
    return fetch(url, {
      method,
      headers: { 'Cache-Control': 'no-cache' },
      signal: AbortSignal.timeout(Math.max(1, Math.min(requestTimeoutMs, remaining))),
    });
  }
  const results = await Promise.all(packages.map(async ({ name, version }) => {
    let reason = 'not checked';
    for (let attempt = 1; Date.now() < deadline; attempt++) {
      try {
        const url = new URL(`${registry}/${encodeURIComponent(name)}/${encodeURIComponent(version)}`);
        url.searchParams.set('postflight', `${Date.now()}-${attempt}`);
        const response = await request(url);
        if (!response.ok) {
          await response.body?.cancel();
          throw new Error(`metadata HTTP ${response.status}`);
        }
        const metadata = await response.json();
        if (metadata.name !== name || metadata.version !== version || !metadata.dist?.tarball) {
          throw new Error('version document does not match the requested package');
        }
        const tarball = await request(metadata.dist.tarball, 'HEAD');
        if (!tarball.ok) throw new Error(`tarball HTTP ${tarball.status}`);
        log(`${name}@${version} is publicly downloadable`);
        return null;
      } catch (error) {
        reason = error.message;
      }
      const remaining = deadline - Date.now();
      if (remaining <= intervalMs) break;
      await sleep(intervalMs);
    }
    return `${name}@${version}: ${reason}`;
  }));
  const failures = results.filter(Boolean);
  if (failures.length) throw new Error(`Publication check failed:\n${failures.join('\n')}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.argv.length < 3) throw new Error('Pass one or more package.json paths');
    const packages = await Promise.all(process.argv.slice(2).map(async path => {
      const { name, version } = JSON.parse(await readFile(path, 'utf8'));
      if (!name || !version) throw new Error(`Missing package name or version: ${path}`);
      return { name, version };
    }));
    await waitForPackages(packages);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
