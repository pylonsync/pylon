/**
 * Outbound network glue. Painting emits cells fast (drag-paint); this
 * batches them into placeTiles / bulldozeTiles calls — one in-flight
 * per kind, re-queued on failure so a dropped call never desyncs the
 * map. Inbound state (the Tile + City rows) arrives via live queries
 * in page.tsx and is pushed into the game through setters.
 */
import { callFn } from "@pylonsync/react";
import type { Cell, EventBus, FrameCtx, GameSystem, TileKind } from "./engine";
import { cellKey } from "./grid";

const FLUSH_MS = 110;

export class Net implements GameSystem {
  readonly name = "net";

  // Pending placements grouped by kind, plus pending bulldozes.
  private place = new Map<TileKind, Map<string, Cell>>();
  private clear = new Map<string, Cell>();
  private placeInFlight = false;
  private clearInFlight = false;
  private lastFlush = 0;
  private mutWindow: number[] = [];

  constructor(
    events: EventBus,
    private readonly cityId: string,
  ) {
    events.on("tilesPainted", ({ kind, cells }) => {
      let m = this.place.get(kind);
      if (!m) {
        m = new Map();
        this.place.set(kind, m);
      }
      for (const c of cells) {
        m.set(cellKey(c.gx, c.gz), c);
        // A freshly painted cell shouldn't also be queued for bulldoze.
        this.clear.delete(cellKey(c.gx, c.gz));
      }
    });
    events.on("tilesBulldozed", ({ cells }) => {
      for (const c of cells) {
        this.clear.set(cellKey(c.gx, c.gz), c);
        for (const m of this.place.values()) m.delete(cellKey(c.gx, c.gz));
      }
    });
  }

  get mutPerSec(): number {
    const now = Date.now();
    this.mutWindow = this.mutWindow.filter((t) => now - t < 1000);
    return this.mutWindow.length;
  }

  private track(p: Promise<unknown>): Promise<unknown> {
    this.mutWindow.push(Date.now());
    return p;
  }

  update(ctx: FrameCtx): void {
    const now = ctx.time * 1000;
    if (now - this.lastFlush < FLUSH_MS) return;
    this.lastFlush = now;

    if (!this.placeInFlight) {
      for (const [kind, m] of this.place) {
        if (m.size === 0) continue;
        const cells = [...m.values()].slice(0, 96).map((c) => ({ gx: c.gx, gz: c.gz, kind }));
        const keys = cells.map((c) => cellKey(c.gx, c.gz));
        this.placeInFlight = true;
        this.track(callFn("placeTiles", { cityId: this.cityId, cells }))
          .then(() => {
            for (const k of keys) m.delete(k);
          })
          .catch(() => {
            /* leave queued; next flush retries */
          })
          .finally(() => {
            this.placeInFlight = false;
          });
        break; // one kind per flush keeps calls small
      }
    }

    if (!this.clearInFlight && this.clear.size > 0) {
      const cells = [...this.clear.values()].slice(0, 96).map((c) => ({ gx: c.gx, gz: c.gz }));
      const keys = cells.map((c) => cellKey(c.gx, c.gz));
      this.clearInFlight = true;
      this.track(callFn("bulldozeTiles", { cityId: this.cityId, cells }))
        .then(() => {
          for (const k of keys) this.clear.delete(k);
        })
        .catch(() => {})
        .finally(() => {
          this.clearInFlight = false;
        });
    }
  }

  dispose(): void {
    this.place.clear();
    this.clear.clear();
  }
}
