// An in-memory stand-in for a mutation ctx (db + scheduler + env), enough to
// run lib/store.ts against. Supports the filter shapes the app uses:
// equality, { $gte }, { $gt }, { $lt }, { $in }, $order, $limit.

type Row = Record<string, unknown> & { id: string };

function matches(row: Row, filter: Record<string, unknown>): boolean {
  for (const [key, cond] of Object.entries(filter)) {
    if (key.startsWith("$")) continue;
    const value = row[key];
    if (cond !== null && typeof cond === "object" && !Array.isArray(cond)) {
      const c = cond as Record<string, unknown>;
      if ("$gte" in c && !(String(value ?? "") >= String(c.$gte))) return false;
      if ("$gt" in c && !(String(value ?? "") > String(c.$gt))) return false;
      if ("$lt" in c && !(String(value ?? "") < String(c.$lt))) return false;
      if ("$in" in c && !(c.$in as unknown[]).includes(value)) return false;
    } else if ((value ?? null) !== (cond ?? null)) {
      return false;
    }
  }
  return true;
}

export function fakeCtx(env: Record<string, string> = {}) {
  const tables = new Map<string, Row[]>();
  let seq = 0;
  const scheduled: { delayMs: number; fn: string; args: Record<string, unknown> }[] = [];
  const table = (name: string) => {
    if (!tables.has(name)) tables.set(name, []);
    return tables.get(name)!;
  };
  const db = {
    async get(entity: string, id: string) {
      return table(entity).find((r) => r.id === id) ?? null;
    },
    async lookup(entity: string, field: string, value: string) {
      return table(entity).find((r) => r[field] === value) ?? null;
    },
    async list(entity: string) {
      return [...table(entity)];
    },
    async query(entity: string, filter: Record<string, unknown>) {
      let rows = table(entity).filter((r) => matches(r, filter));
      const order = filter.$order as Record<string, "asc" | "desc"> | undefined;
      if (order) {
        const [[field, dir]] = Object.entries(order);
        rows = rows.sort((a, b) => {
          const x = String(a[field] ?? "");
          const y = String(b[field] ?? "");
          return (x < y ? -1 : x > y ? 1 : 0) * (dir === "desc" ? -1 : 1);
        });
      }
      if (typeof filter.$limit === "number") rows = rows.slice(0, filter.$limit);
      return rows;
    },
    async insert(entity: string, data: Record<string, unknown>) {
      seq += 1;
      const id = `${entity.toLowerCase()}_${seq}`;
      table(entity).push({ ...data, id });
      return id;
    },
    async update(entity: string, id: string, patch: Record<string, unknown>) {
      const row = table(entity).find((r) => r.id === id);
      if (!row) return false;
      Object.assign(row, patch);
      return true;
    },
    async delete(entity: string, id: string) {
      const rows = table(entity);
      const i = rows.findIndex((r) => r.id === id);
      if (i === -1) return false;
      rows.splice(i, 1);
      return true;
    },
  };
  const scheduler = {
    async runAfter(delayMs: number, fn: string, args: Record<string, unknown>) {
      scheduled.push({ delayMs, fn, args });
      return `sched_${scheduled.length}`;
    },
    async runAt(at: number, fn: string, args: Record<string, unknown>) {
      scheduled.push({ delayMs: at - Date.now(), fn, args });
      return `sched_${scheduled.length}`;
    },
    async cancel() {},
  };
  return { ctx: { db, scheduler, env } as never, tables, scheduled, table };
}
