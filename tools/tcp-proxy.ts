// A TCP proxy for tests: `bun tools/tcp-proxy.ts <listen port> <target host> <target port>`.
// tools/smoke-shard-cluster.sh puts it between one machine and Postgres. On
// SIGUSR1 it drops every byte from then on and keeps every socket open, so
// the machine's database calls hang instead of failing.

const [listenPort, targetHost, targetPort] = process.argv.slice(2);
if (!listenPort || !targetHost || !targetPort) {
  console.error("usage: bun tools/tcp-proxy.ts <listen port> <target host> <target port>");
  process.exit(2);
}

type Pair = { upstream?: Bun.Socket<Pair>; pending: Uint8Array[] };

let dropping = false;
process.on("SIGUSR1", () => {
  dropping = true;
  console.log("dropping all traffic");
});

Bun.listen<Pair>({
  hostname: "127.0.0.1",
  port: Number(listenPort),
  socket: {
    open(client) {
      client.data = { pending: [] };
      Bun.connect<Pair>({
        hostname: targetHost,
        port: Number(targetPort),
        socket: {
          open(upstream) {
            upstream.data = { upstream: client, pending: [] };
            client.data.upstream = upstream;
            for (const chunk of client.data.pending) upstream.write(chunk);
            client.data.pending = [];
          },
          data(upstream, chunk) {
            if (!dropping) upstream.data.upstream?.write(chunk);
          },
          close(upstream) {
            upstream.data.upstream?.end();
          },
          error(upstream) {
            upstream.data.upstream?.end();
          },
        },
      }).catch(() => client.end());
    },
    data(client, chunk) {
      if (dropping) return;
      if (client.data.upstream) client.data.upstream.write(chunk);
      else client.data.pending.push(new Uint8Array(chunk));
    },
    close(client) {
      client.data.upstream?.end();
    },
    error(client) {
      client.data.upstream?.end();
    },
  },
});
console.log(`proxying 127.0.0.1:${listenPort} -> ${targetHost}:${targetPort}`);
