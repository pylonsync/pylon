// The local replica answers a filter the way the server does. The cases in
// query-filter-cases.json also run against the server
// (crates/runtime/tests/query_filter_parity.rs), so both sides are held to
// the same answers.

import { expect, spyOn, test } from "bun:test";
import cases from "./query-filter-cases.json";
import { compareForOrder, compileWhere } from "./queryFilter";

for (const { where, keys } of cases.cases) {
  test(`where ${JSON.stringify(where)}`, () => {
    const match = compileWhere(where);
    const got = cases.rows.filter((r) => match(r)).map((r) => r.key);
    expect(got).toEqual(keys);
  });
}

test("an operator that cannot apply matches nothing and warns once in dev", () => {
  const warn = spyOn(console, "warn").mockImplementation(() => {});
  const match = compileWhere({ rank: { $gt: "2" } });
  expect(cases.rows.filter((r) => match(r))).toEqual([]);
  expect(cases.rows.filter((r) => match(r))).toEqual([]);
  expect(warn).toHaveBeenCalledTimes(1);
  expect(String(warn.mock.calls[0]?.[0])).toContain("where.rank.$gt");

  const unknown = compileWhere({ rank: { $between: [1, 2] } });
  expect(cases.rows.filter((r) => unknown(r))).toEqual([]);
  expect(warn).toHaveBeenCalledTimes(2);
  warn.mockRestore();
});

test("ordering follows the server: code units, not locale; datetimes by time", () => {
  const titles = ["banana", "Banana", "apple", "cherry"];
  expect([...titles].sort((a, b) => compareForOrder(a, b, "asc"))).toEqual([
    "Banana",
    "apple",
    "banana",
    "cherry",
  ]);
  const times = ["2026-10-01T12:00:00Z", "2026-10-01T08:00:00-05:00"];
  expect([...times].sort((a, b) => compareForOrder(a, b, "asc"))).toEqual([
    "2026-10-01T12:00:00Z",
    "2026-10-01T08:00:00-05:00",
  ]);
});
