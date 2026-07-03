// The client ↔ server doc-shape contract. The server (crates/crdt) puts
// every entity field inside ONE root LoroMap named "row" — a
// `crdt: "text"` field is a LoroText CHILD of that map, seeded at insert
// and projected back to SQL after every merge. The client accessors must
// resolve through that map: the old top-level `doc.getText(field)` put
// edits in a container the server never reads, so seeded content rendered
// empty in the editor and typed content never projected to the row.
import { describe, expect, test } from "bun:test";
import { LoroDoc, LoroText } from "loro-crdt";
import { getLoroText, getLoroMap } from "./index";

/** Build a doc the way the server's apply_patch does: root "row" map
 *  with a seeded Text child for the field. */
function serverSeededDoc(field: string, initial: string): LoroDoc {
  const doc = new LoroDoc();
  const row = doc.getMap("row");
  const text = row.setContainer(field, new LoroText());
  text.insert(0, initial);
  doc.commit();
  return doc;
}

describe("row-map contract", () => {
  test("getLoroText resolves the server-seeded container (initial value visible)", () => {
    const doc = serverSeededDoc("content", "# Welcome");
    const text = getLoroText(doc, "content");
    expect(text.toString()).toBe("# Welcome");
  });

  test("edits land inside the row map where the server projects from", () => {
    const doc = serverSeededDoc("content", "abc");
    const text = getLoroText(doc, "content");
    text.insert(3, "def");
    doc.commit();
    // The server reads doc.getMap("row").get(field) — the edit must be
    // visible THERE, not in a top-level container.
    const json = doc.toJSON() as { row?: { content?: string } };
    expect(json.row?.content).toBe("abcdef");
    // And nothing leaked into the legacy top-level namespace.
    expect(doc.getText("content").toString()).toBe("");
  });

  test("creates the container under row/ for rows that predate CRDT seeding", () => {
    const doc = new LoroDoc();
    const text = getLoroText(doc, "content");
    text.insert(0, "x");
    doc.commit();
    const json = doc.toJSON() as { row?: { content?: string } };
    expect(json.row?.content).toBe("x");
  });

  test("edits after a catch-up import land in the winning container", () => {
    // The race: a component creates the field container eagerly BEFORE the
    // server's catch-up snapshot arrives; the merge then makes one of the
    // two containers win the row-map key LWW. Edits must follow the WINNER
    // (what row.content projects to), not a handle captured pre-merge —
    // pre-fix, every keystroke went into the orphaned loser and the server
    // never saw it.
    const server = serverSeededDoc("content", "seeded");
    const snapshot = server.export({ mode: "snapshot" });

    const client = new LoroDoc();
    getLoroText(client, "content").insert(0, "early");
    client.commit();
    client.import(snapshot);

    const live = getLoroText(client, "content");
    live.insert(live.length, "!");
    client.commit();
    const json = client.toJSON() as { row?: { content?: string } };
    expect(json.row?.content?.endsWith("!")).toBe(true);
  });

  test("getLoroMap resolves a field-level map child of row/", () => {
    const doc = new LoroDoc();
    const m = getLoroMap(doc, "meta");
    m.set("k", "v");
    doc.commit();
    const json = doc.toJSON() as { row?: { meta?: { k?: string } } };
    expect(json.row?.meta?.k).toBe("v");
  });
});
