import { afterEach, expect, spyOn, test } from "bun:test";
import { act, cleanup, render, screen } from "@testing-library/react";
import { SyncEngine } from "@pylonsync/sync";
import { useQuery, useQueryOne, useQueryRaw, useQueryOneRaw } from "./hooks";

afterEach(cleanup);

test("single-row hooks do not read unchanged rows in the same entity", () => {
  const engine = makeEngine();
  for (let i = 0; i < 1000; i++) {
    engine.store.optimisticInsertWithId("Item", String(i), { text: "x".repeat(10_000) });
  }
  const raw = Array.from({ length: 1000 }, (_, i) => useQueryOneRaw(engine, "Item", String(i)));
  const changed: number[] = [];
  const stops = raw.map((row, i) => row.subscribe(() => changed.push(i)));
  function RowProbe() {
    const row = useQueryOne<{ text: string }>(engine, "Item", "999");
    return <div data-testid="row-only">{row.data?.text}</div>;
  }
  render(<RowProbe />);
  const get = spyOn(engine.store, "get");
  act(() => engine.store.optimisticUpdate("Item", "0", { text: "changed" }));
  expect(get).toHaveBeenCalledTimes(1);
  expect(changed).toEqual([0]);
  const previous = raw[999].getSnapshot();
  act(() => engine.store.optimisticDelete("Item", "999"));
  expect(raw[999].getSnapshot()).toBeNull();
  expect(screen.getByTestId("row-only").textContent).toBe("");
  act(() => engine.store.restoreRow("Item", "999", previous));
  expect(raw[999].getSnapshot()).toEqual(previous);
  expect(screen.getByTestId("row-only").textContent).toHaveLength(10_000);
  act(() => engine.store.clearAll());
  expect(raw.every((row) => row.getSnapshot() === null)).toBe(true);
  expect(screen.getByTestId("row-only").textContent).toBe("");
  stops.forEach((stop) => stop());
  get.mockRestore();
});

function Probe({ engine, entity = "Todo", id = "1", limit = 20 }: {
  engine: SyncEngine; entity?: string; id?: string; limit?: number;
}) {
  const list = useQuery<{ id: string; title?: string }>(engine, entity, { orderBy: { id: "asc" }, limit });
  const one = useQueryOne<{ id: string; title?: string }>(engine, entity, id);
  return <>
    <div data-testid="rows">{JSON.stringify(list.data)}</div>
    <div data-testid="one">{JSON.stringify(one.data)}</div>
    <div data-testid="status">{`${list.loading}:${list.synced}:${one.loading}:${one.synced}`}</div>
  </>;
}

function makeEngine() {
  const engine = new SyncEngine({ baseUrl: "http://test.invalid", multiTab: false, persist: false });
  engine.store.optimisticInsertWithId("Todo", "1", { title: "first" });
  engine.store.optimisticInsertWithId("Todo", "2", { title: "second" });
  return engine;
}

test("unrelated changes and parent renders do not scan cached query rows", () => {
  const engine = makeEngine();
  const list = spyOn(engine.store, "list");
  const get = spyOn(engine.store, "get");
  const view = render(<Probe engine={engine} />);
  list.mockClear();
  get.mockClear();
  act(() => engine.store.optimisticInsertWithId("Other", "1", { title: "other" }));
  view.rerender(<Probe engine={engine} />);
  expect(list).not.toHaveBeenCalled();
  expect(get).not.toHaveBeenCalled();
  act(() => engine.store.optimisticUpdate("Todo", "1", { title: "changed" }));
  expect(list).toHaveBeenCalledTimes(1);
  expect(get).toHaveBeenCalledTimes(1);
  expect(screen.getByTestId("one").textContent).toContain("changed");
  view.rerender(<Probe engine={engine} limit={1} />);
  expect(JSON.parse(screen.getByTestId("rows").textContent!)).toHaveLength(1);
  expect(list).toHaveBeenCalledTimes(2);
});

test("global sync signals update status without rebuilding row snapshots", () => {
  const engine = makeEngine();
  const list = spyOn(engine.store, "list");
  render(<Probe engine={engine} />);
  list.mockClear();
  act(() => {
    const state = engine as unknown as { _synced: boolean; _initialSyncSettled: boolean };
    state._synced = true;
    state._initialSyncSettled = true;
    engine.store.notify();
  });
  expect(screen.getByTestId("status").textContent).toBe("false:true:false:true");
  expect(list).not.toHaveBeenCalled();
  act(() => {
    const state = engine as unknown as { _synced: boolean; _initialSyncSettled: boolean };
    state._synced = false;
    state._initialSyncSettled = false;
    engine.store.clearAll();
  });
  expect(screen.getByTestId("rows").textContent).toBe("[]");
  expect(screen.getByTestId("one").textContent).toBe("null");
  expect(screen.getByTestId("status").textContent).toBe("true:false:true:false");
});

test("snapshot cache tracks row ID, entity, and engine changes", () => {
  const engine = makeEngine();
  engine.store.optimisticInsertWithId("Other", "2", { title: "other" });
  const view = render(<Probe engine={engine} />);
  view.rerender(<Probe engine={engine} id="2" />);
  expect(screen.getByTestId("one").textContent).toContain("second");
  view.rerender(<Probe engine={engine} entity="Other" id="2" />);
  expect(screen.getByTestId("one").textContent).toContain("other");
  const next = makeEngine();
  next.store.optimisticUpdate("Todo", "1", { title: "next engine" });
  view.rerender(<Probe engine={next} />);
  expect(screen.getByTestId("one").textContent).toContain("next engine");
});

test("raw query subscribers skip unrelated changes and global status signals", () => {
  const engine = makeEngine();
  const list = spyOn(engine.store, "list");
  const get = spyOn(engine.store, "get");
  const rows = useQueryRaw(engine, "Todo");
  const row = useQueryOneRaw(engine, "Todo", "1");
  let updates = 0;
  const stopRows = rows.subscribe(() => updates++);
  const stopRow = row.subscribe(() => updates++);
  list.mockClear();
  get.mockClear();
  engine.store.optimisticInsertWithId("Other", "1", {});
  engine.store.notify();
  expect(list).not.toHaveBeenCalled();
  expect(get).not.toHaveBeenCalled();
  engine.store.optimisticUpdate("Todo", "1", { title: "changed" });
  expect(updates).toBe(2);
  expect(row.getSnapshot()?.title).toBe("changed");
  stopRows();
  stopRow();
});

test("filters with values omitted by JSON retain their query behavior", () => {
  const engine = makeEngine();
  engine.store.optimisticUpdate("Todo", "1", { rank: null });
  function FilterProbe({ where }: { where: Record<string, unknown> }) {
    const query = useQuery(engine, "Todo", { where });
    return <div data-testid="filtered">{query.data.length}</div>;
  }
  const view = render(<FilterProbe where={{}} />);
  expect(screen.getByTestId("filtered").textContent).toBe("2");
  view.rerender(<FilterProbe where={{ title: undefined }} />);
  expect(screen.getByTestId("filtered").textContent).toBe("0");
  view.rerender(<FilterProbe where={{}} />);
  expect(screen.getByTestId("filtered").textContent).toBe("2");
  view.rerender(<FilterProbe where={{ rank: null }} />);
  expect(screen.getByTestId("filtered").textContent).toBe("1");
  view.rerender(<FilterProbe where={{ rank: Number.NaN }} />);
  expect(screen.getByTestId("filtered").textContent).toBe("0");
  view.rerender(<FilterProbe where={{ rank: Number.POSITIVE_INFINITY }} />);
  expect(screen.getByTestId("filtered").textContent).toBe("0");
  view.rerender(<FilterProbe where={{ rank: null }} />);
  expect(screen.getByTestId("filtered").textContent).toBe("1");
  const date = new Date("2026-01-01T00:00:00.000Z");
  view.rerender(<FilterProbe where={{ rank: date.toISOString() }} />);
  expect(screen.getByTestId("filtered").textContent).toBe("0");
  view.rerender(<FilterProbe where={{ rank: date }} />);
  expect(screen.getByTestId("filtered").textContent).toBe("2");
  view.rerender(<FilterProbe where={{ rank: date.toISOString() }} />);
  expect(screen.getByTestId("filtered").textContent).toBe("0");
});

test("membership filters retain includes semantics and combine with other operators", () => {
  const engine = makeEngine();
  const ref = { shared: true };
  const values = [undefined, null, NaN, -0, "zero", ref, { shared: true }, 2];
  values.forEach((value, i) => engine.store.optimisticInsertWithId("Value", String(i), { value }));
  function FilterProbe({ where }: { where: Record<string, unknown> }) {
    const query = useQuery(engine, "Value", { where });
    return <div data-testid="matches">{query.data.map((row) => row.id).join(",")}</div>;
  }
  const view = render(<FilterProbe where={{ value: { $in: [] } }} />);
  for (const members of [[], [NaN, NaN], [0], [ref], [null], [undefined], new Array(2), [2, "zero"]]) {
    view.rerender(<FilterProbe where={{ value: { $in: members } }} />);
    expect(screen.getByTestId("matches").textContent).toBe(
      values.flatMap((value, i) => members.includes(value) ? [String(i)] : []).join(","),
    );
  }
  view.rerender(<FilterProbe where={{ value: { $in: [0, 2], $gt: 0, $lte: 2, $not: 3 } }} />);
  expect(screen.getByTestId("matches").textContent).toBe("7");
  view.rerender(<FilterProbe where={{ value: { $in: "zero" } }} />);
  expect(screen.getByTestId("matches").textContent).toBe("");
});

test("membership filters read the search list once per query evaluation", () => {
  const engine = makeEngine();
  const members = Array.from({ length: 1000 }, (_, i) => `missing-${i}`);
  let reads = 0;
  Object.defineProperty(members, 0, { get() { reads++; return "missing-0"; } });
  const rows = Array.from({ length: 20_000 }, (_, i) => ({ id: String(i), title: `row-${i}` }));
  engine.store.applyReconcileInMemory("Todo", rows, [], 1);
  function FilterProbe() {
    const query = useQuery(engine, "Todo", { where: { title: { $in: members } } });
    return <div data-testid="matches">{query.data.length}</div>;
  }
  render(<FilterProbe />);
  expect(screen.getByTestId("matches").textContent).toBe("0");
  // Query-key generation also reads the array. It must not be read per row.
  expect(reads).toBeLessThan(20);
  reads = 0;
  act(() => engine.store.optimisticUpdate("Todo", "1", { title: "missing-1" }));
  expect(screen.getByTestId("matches").textContent).toBe("1");
  expect(reads).toBeLessThan(20);
});


test("unordered limited queries stop after enough matches and preserve slice limits", () => {
  const engine = makeEngine();
  let reads = 0;
  const rows = Array.from({ length: 20_000 }, (_, i) => ({
    id: String(i),
    get group() { reads++; return i % 2 === 0 ? "yes" : "no"; },
  }));
  const list = spyOn(engine.store, "list").mockReturnValue(rows);
  let visits = 0;
  const iterator = spyOn(engine.store, "rows").mockImplementation(function* () {
    for (const row of rows) {
      visits++;
      yield row;
    }
  });
  function LimitProbe({ limit, sorted = false }: { limit: number; sorted?: boolean }) {
    const query = useQuery(engine, "Todo", {
      where: { group: "yes" }, limit,
      orderBy: sorted ? { id: "desc" } : {},
    });
    return <div data-testid="limited">{query.data.map((row) => row.id).join(",")}</div>;
  }
  const view = render(<LimitProbe limit={10} />);
  expect(screen.getByTestId("limited").textContent).toBe("0,2,4,6,8,10,12,14,16,18");
  expect(reads).toBeLessThan(50);
  expect(visits).toBe(19);
  expect(list).not.toHaveBeenCalled();
  const matches = rows.filter((row) => row.group === "yes").map((row) => row.id);
  for (const limit of [0, 1, 1.9, -1, Infinity, NaN]) {
    view.rerender(<LimitProbe limit={limit} />);
    expect(screen.getByTestId("limited").textContent).toBe(matches.slice(0, limit).join(","));
  }
  view.rerender(<LimitProbe limit={10} sorted />);
  const sorted = matches.slice().sort((a, b) => b.localeCompare(a));
  expect(screen.getByTestId("limited").textContent).toBe(sorted.slice(0, 10).join(","));
  list.mockRestore();
  iterator.mockRestore();
});

test("equal JSON filter operands keep their distinct reference semantics", () => {
  const engine = makeEngine();
  const a = { shared: true };
  const b = { shared: true };
  const firstArray = [1];
  const secondArray = [1];
  for (const [id, value] of [["a", a], ["b", b], ["c", firstArray], ["d", secondArray]] as const) {
    engine.store.optimisticInsertWithId("Value", id, { value });
  }
  function RefProbe({ where }: { where: Record<string, unknown> }) {
    const query = useQuery(engine, "Value", { where });
    return <div data-testid="references">{query.data.map((row) => row.id).join(",")}</div>;
  }
  const view = render(<RefProbe where={{ value: { $in: [a] } }} />);
  expect(screen.getByTestId("references").textContent).toBe("a");
  view.rerender(<RefProbe where={{ value: { $in: [b] } }} />);
  expect(screen.getByTestId("references").textContent).toBe("b");
  view.rerender(<RefProbe where={{ value: { $not: a } }} />);
  expect(screen.getByTestId("references").textContent).toBe("b,c,d");
  view.rerender(<RefProbe where={{ value: { $not: b } }} />);
  expect(screen.getByTestId("references").textContent).toBe("a,c,d");
  view.rerender(<RefProbe where={{ value: firstArray }} />);
  expect(screen.getByTestId("references").textContent).toBe("c");
  view.rerender(<RefProbe where={{ value: secondArray }} />);
  expect(screen.getByTestId("references").textContent).toBe("d");
});
