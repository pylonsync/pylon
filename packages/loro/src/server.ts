/**
 * A row's live CRDT document in a server function (an action), through
 * `ctx.crdt`:
 *
 *   import { openLiveDoc } from "@pylonsync/loro/server";
 *
 *   const live = await openLiveDoc(ctx, "Doc", id);
 *   await live.edit((doc) => doc.getMap("row").set("status", "done"));
 *
 * `edit` pulls in what others wrote since the doc was opened, runs the
 * change, and sends only this function's ops. Everyone editing sees them as
 * they land. For a ProseMirror field, `editProseMirror` in
 * `@pylonsync/loro/prosemirror` does the same with a ProseMirror document.
 *
 * Each `openLiveDoc` is a new Loro peer, so two functions editing one row
 * at once merge like two people do.
 */
import { LoroDoc } from "loro-crdt";
import { exportOwnOps, ownCounter } from "./outbox";

/** The byte operations `ctx.crdt` provides (`@pylonsync/functions` `Crdt`). */
export interface CrdtOps {
  snapshot(entity: string, id: string): Promise<Uint8Array | null>;
  since(entity: string, id: string, version: Uint8Array): Promise<Uint8Array | null>;
  apply(entity: string, id: string, update: Uint8Array): Promise<void>;
}

export interface LiveDocContext {
  crdt: CrdtOps & { unsafe: CrdtOps };
}

export interface OpenLiveDocOptions {
  /** Server trust even under strict function policies (`ctx.crdt.unsafe`). */
  unsafe?: boolean;
}

export class LiveDoc {
  readonly doc: LoroDoc;
  readonly entity: string;
  readonly id: string;
  private readonly ops: CrdtOps;
  /** This peer's op counter the server has merged (exclusive end). */
  private confirmed = 0;

  private constructor(ops: CrdtOps, entity: string, id: string, doc: LoroDoc) {
    this.ops = ops;
    this.entity = entity;
    this.id = id;
    this.doc = doc;
  }

  /** Open the row's document. Throws NOT_FOUND when there is none to open. */
  static async open(ctx: LiveDocContext, entity: string, id: string, options: OpenLiveDocOptions = {}): Promise<LiveDoc> {
    const ops = options.unsafe ? ctx.crdt.unsafe : ctx.crdt;
    const snapshot = await ops.snapshot(entity, id);
    if (!snapshot) {
      throw Object.assign(new Error(`${entity} ${id} has no document to open`), { code: "NOT_FOUND" });
    }
    const doc = new LoroDoc();
    doc.import(snapshot);
    return new LiveDoc(ops, entity, id, doc);
  }

  /** Pull in what others wrote since this doc last heard from the server. */
  async refresh(): Promise<void> {
    const update = await this.ops.since(this.entity, this.id, this.doc.oplogVersion().encode());
    if (update && update.length > 0) this.doc.import(update);
  }

  /**
   * Send this function's ops the server has not merged yet. Commits pending
   * changes first. Returns false when there was nothing to send.
   */
  async commit(): Promise<boolean> {
    this.doc.commit();
    const target = ownCounter(this.doc);
    if (target <= this.confirmed) return false;
    await this.ops.apply(this.entity, this.id, exportOwnOps(this.doc, this.confirmed));
    this.confirmed = target;
    return true;
  }

  /** Refresh, run `change` on the doc, and send the result. */
  async edit(change: (doc: LoroDoc) => void | Promise<void>): Promise<void> {
    await this.refresh();
    await change(this.doc);
    await this.commit();
  }
}

/** See {@link LiveDoc.open}. */
export function openLiveDoc(
  ctx: LiveDocContext,
  entity: string,
  id: string,
  options: OpenLiveDocOptions = {},
): Promise<LiveDoc> {
  return LiveDoc.open(ctx, entity, id, options);
}
