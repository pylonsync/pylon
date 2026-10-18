import { describe, expect, test } from "bun:test";
import { entitiesToManifest, entity, field } from "./index";

describe("entity afterEdit option", () => {
  test("reaches the manifest as after_edit, with the default window", () => {
    const [a, b] = entitiesToManifest([
      entity("Doc", { title: field.string() }, { afterEdit: { fn: "indexDoc" } }),
      entity("Note", { title: field.string() }, { afterEdit: { fn: "indexNote", delaySeconds: 30 } }),
    ]);
    expect(a.after_edit).toEqual({ function: "indexDoc", delay_secs: 5 });
    expect(b.after_edit).toEqual({ function: "indexNote", delay_secs: 30 });
  });

  test("is omitted when unset", () => {
    const [m] = entitiesToManifest([entity("Doc", { title: field.string() })]);
    expect("after_edit" in m).toBe(false);
  });

  test("refuses a window that is not a whole number of seconds, and a missing function", () => {
    for (const delaySeconds of [0, 1.5, -1]) {
      expect(() =>
        entitiesToManifest([entity("Doc", { t: field.string() }, { afterEdit: { fn: "f", delaySeconds } })]),
      ).toThrow("whole number of seconds");
    }
    expect(() => entitiesToManifest([entity("Doc", { t: field.string() }, { afterEdit: { fn: "" } })])).toThrow(
      "afterEdit needs fn",
    );
  });
});
