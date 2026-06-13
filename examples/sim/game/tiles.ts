/**
 * TileMap — the bridge between synced Tile rows and the 3-D scene.
 *
 * setTiles() is called whenever the live query updates. It diffs the
 * incoming rows against what's rendered and does the minimum work:
 *   - roads  → one merged auto-tiled mesh, rebuilt only when the road
 *              set changes
 *   - lots   → one merged coloured-pad mesh per zone-set change
 *   - builds → one Object3D per zone tile with level>0, (re)made when
 *              its level changes, with a rise animation
 *
 * The whole city is deterministic from the row set, so every client
 * renders an identical skyline.
 */
import * as THREE from "three";
import { TILE, ZONE } from "./config";
import type { Cell, EventBus, FrameCtx, GameSystem, TileKind } from "./engine";
import { cellCenterX, cellCenterZ, cellKey } from "./grid";
import { type BuildingZone, makeBuilding } from "./kit";
import { buildRoadMesh } from "./roads";

interface TileState {
  kind: TileKind;
  level: number;
}

interface BuildingView {
  level: number;
  obj: THREE.Object3D;
  riseT: number; // 0→1 grow-in
}

const LOT_Y = 0.05;

export class TileMap implements GameSystem {
  readonly name = "tiles";
  readonly group = new THREE.Group();

  private readonly roadGroup = new THREE.Group();
  private readonly lotGroup = new THREE.Group();
  private readonly buildGroup = new THREE.Group();

  private state = new Map<string, TileState>();
  private buildings = new Map<string, BuildingView>();
  private rising = new Set<BuildingView>();
  private roadSig = "";
  private lotSig = "";

  constructor(private readonly events: EventBus) {
    this.group.add(this.roadGroup, this.lotGroup, this.buildGroup);
  }

  /** Current kind at a cell (for placement validation / cursor). */
  kindAt(gx: number, gz: number): TileKind | null {
    return this.state.get(cellKey(gx, gz))?.kind ?? null;
  }

  get tileCount(): number {
    return this.state.size;
  }

  /** Apply a full snapshot of tiles from the live query. */
  setTiles(rows: Array<{ gx: number; gz: number; kind: string; level: number }>): void {
    const next = new Map<string, TileState>();
    for (const r of rows) {
      next.set(cellKey(r.gx, r.gz), { kind: r.kind as TileKind, level: r.level ?? 0 });
    }
    this.state = next;

    // --- Roads: rebuild only if the road set changed ---
    const roadCells: Cell[] = [];
    for (const [key, t] of next) {
      if (t.kind === "road") {
        const [gx, gz] = key.split(",").map(Number);
        roadCells.push({ gx, gz });
      }
    }
    const roadSig = roadCells
      .map((c) => c.gx + ":" + c.gz)
      .sort()
      .join("|");
    if (roadSig !== this.roadSig) {
      this.roadSig = roadSig;
      this.roadGroup.clear();
      if (roadCells.length) {
        const isRoad = (gx: number, gz: number) => next.get(cellKey(gx, gz))?.kind === "road";
        this.roadGroup.add(buildRoadMesh(roadCells, isRoad));
      }
    }

    // --- Zone lots: rebuild on zone-set change ---
    const zoneCells = [...next.entries()].filter(([, t]) => t.kind !== "road");
    const lotSig = zoneCells
      .map(([k, t]) => k + t.kind)
      .sort()
      .join("|");
    if (lotSig !== this.lotSig) {
      this.lotSig = lotSig;
      this.lotGroup.clear();
      if (zoneCells.length) this.lotGroup.add(this.buildLots(zoneCells));
    }

    // --- Buildings: per zone tile with level>0 ---
    const wanted = new Set<string>();
    for (const [key, t] of next) {
      if (t.kind === "road" || t.level < 1) continue;
      wanted.add(key);
      const [gx, gz] = key.split(",").map(Number);
      const existing = this.buildings.get(key);
      if (!existing || existing.level !== t.level) {
        if (existing) this.buildGroup.remove(existing.obj);
        const obj = makeBuilding(t.kind as BuildingZone, t.level, gx, gz);
        obj.position.set(cellCenterX(gx), 0, cellCenterZ(gz));
        obj.scale.y = 0.04;
        this.buildGroup.add(obj);
        const view: BuildingView = { level: t.level, obj, riseT: 0 };
        this.buildings.set(key, view);
        this.rising.add(view);
        this.events.emit("buildingRose", {
          point: new THREE.Vector3(cellCenterX(gx), 1, cellCenterZ(gz)),
        });
      }
    }
    // Remove buildings whose tile is gone or de-leveled.
    for (const [key, view] of this.buildings) {
      if (!wanted.has(key)) {
        this.buildGroup.remove(view.obj);
        this.rising.delete(view);
        this.buildings.delete(key);
      }
    }
  }

  private buildLots(cells: Array<[string, TileState]>): THREE.Mesh {
    const pos: number[] = [];
    const col: number[] = [];
    for (const [key, t] of cells) {
      if (t.kind === "road") continue;
      const [gx, gz] = key.split(",").map(Number);
      const cx = cellCenterX(gx);
      const cz = cellCenterZ(gz);
      const hex = ZONE[t.kind as BuildingZone].lot;
      const c: [number, number, number] = [
        ((hex >> 16) & 0xff) / 255,
        ((hex >> 8) & 0xff) / 255,
        (hex & 0xff) / 255,
      ];
      const s = TILE * 0.46;
      const v = [
        cx - s, LOT_Y, cz - s, cx + s, LOT_Y, cz - s, cx + s, LOT_Y, cz + s,
        cx - s, LOT_Y, cz - s, cx + s, LOT_Y, cz + s, cx - s, LOT_Y, cz + s,
      ];
      pos.push(...v);
      for (let i = 0; i < 6; i++) col.push(c[0], c[1], c[2]);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    geo.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
    geo.computeVertexNormals();
    const mesh = new THREE.Mesh(geo, new THREE.MeshLambertMaterial({ vertexColors: true }));
    mesh.receiveShadow = true;
    return mesh;
  }

  update(ctx: FrameCtx): void {
    if (this.rising.size === 0) return;
    for (const view of Array.from(this.rising)) {
      view.riseT = Math.min(1, view.riseT + ctx.dt * 3.2);
      // easeOutBack for a little settle overshoot.
      const t = view.riseT;
      const s = 1 + 2.2 * Math.pow(t - 1, 3) + 1.2 * Math.pow(t - 1, 2);
      view.obj.scale.y = Math.max(0.04, s);
      if (view.riseT >= 1) {
        view.obj.scale.y = 1;
        this.rising.delete(view);
      }
    }
  }

  dispose(): void {
    this.group.clear();
  }
}
