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
import { type BuildingZone, hasRoadKit, makeAvenueTile, makeBuilding, makeRoadTile } from "./kit";
import { buildRoadMesh } from "./roads";
import { heightAt } from "./terrain";

// Reused when draping road tiles onto the terrain slope (see rebuildRoads).
const ROAD_UP = new THREE.Vector3(0, 1, 0);
const ROAD_UP_NORMAL = new THREE.Vector3();

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

// --- Park props (shared geometry/material; cloned per park) ---
const PARK_LAWN_GEO = new THREE.PlaneGeometry(TILE * 0.96, TILE * 0.96).rotateX(-Math.PI / 2);
// A brighter, mowed green that reads as manicured next to the wilder terrain grass.
const PARK_LAWN_MAT = new THREE.MeshStandardMaterial({ color: 0x6fa24a, roughness: 0.95 });
const FOUNTAIN_BASIN_GEO = new THREE.CylinderGeometry(TILE * 0.17, TILE * 0.19, 0.32, 18).translate(0, 0.16, 0);
const FOUNTAIN_PLINTH_GEO = new THREE.CylinderGeometry(0.12, 0.16, 0.55, 10).translate(0, 0.45, 0);
const FOUNTAIN_WATER_GEO = new THREE.CylinderGeometry(TILE * 0.14, TILE * 0.14, 0.06, 18).translate(0, 0.3, 0);
const STONE_MAT = new THREE.MeshStandardMaterial({ color: 0xc3bdb0, roughness: 0.85 });
const FOUNTAIN_WATER_MAT = new THREE.MeshStandardMaterial({
  color: 0x5aa6cc,
  roughness: 0.1,
  metalness: 0.2,
  emissive: 0x123040,
  emissiveIntensity: 0.25,
});
const TRUNK_GEO = new THREE.CylinderGeometry(0.08, 0.12, 1.3, 6).translate(0, 0.65, 0);
const TRUNK_MAT = new THREE.MeshStandardMaterial({ color: 0x6b4f33, roughness: 0.9 });
const CANOPY_GEO = new THREE.IcosahedronGeometry(0.85, 0).translate(0, 1.75, 0);
const CANOPY_MAT = new THREE.MeshStandardMaterial({ color: 0x4f8a3e, roughness: 0.9, flatShading: true });

/** A small stone fountain (basin + plinth + a translucent water disc). */
function makeFountain(): THREE.Object3D {
  const g = new THREE.Group();
  const basin = new THREE.Mesh(FOUNTAIN_BASIN_GEO, STONE_MAT);
  basin.castShadow = true;
  basin.receiveShadow = true;
  const plinth = new THREE.Mesh(FOUNTAIN_PLINTH_GEO, STONE_MAT);
  plinth.castShadow = true;
  const water = new THREE.Mesh(FOUNTAIN_WATER_GEO, FOUNTAIN_WATER_MAT);
  g.add(basin, plinth, water);
  return g;
}

// --- Parking-lot props ---
const CARPARK_PAD_GEO = new THREE.PlaneGeometry(TILE * 0.94, TILE * 0.94).rotateX(-Math.PI / 2);
const CARPARK_PAD_MAT = new THREE.MeshStandardMaterial({ color: 0x46474b, roughness: 1 });
const CARPARK_LINE_GEO = new THREE.PlaneGeometry(TILE * 0.04, TILE * 0.62).rotateX(-Math.PI / 2);
const CARPARK_LINE_MAT = new THREE.MeshBasicMaterial({ color: 0xd7d4c8 });
const CAR_CABIN_MAT = new THREE.MeshStandardMaterial({ color: 0x2a3038, roughness: 0.4, metalness: 0.2 });
const CAR_COLORS = [
  0x9a3b3b, 0x35507c, 0x5a5e66, 0xb0b3b8, 0x3a6b4a, 0xc7a23a, 0xe2e2e2, 0x2c2c30, 0x7a4a2a,
];

/** A small two-box parked car. The `seed` varies the silhouette (sedan vs a
 *  taller SUV/van) so a lot isn't all identical boxes. */
function makeParkedCar(color: number, seed: number): THREE.Object3D {
  const g = new THREE.Group();
  let r = Math.abs(Math.sin(seed * 12.9898) * 4151.3);
  r -= Math.floor(r);
  const suv = r < 0.4;
  const len = 1.75 + r * 0.55;
  const w = 0.82 + r * 0.16;
  const bodyH = suv ? 0.58 : 0.42;
  const body = new THREE.Mesh(
    new THREE.BoxGeometry(w, bodyH, len),
    new THREE.MeshStandardMaterial({ color, roughness: 0.5, metalness: 0.15, flatShading: true }),
  );
  body.position.y = bodyH / 2;
  body.castShadow = true;
  const cabinH = suv ? 0.4 : 0.34;
  const cabin = new THREE.Mesh(
    new THREE.BoxGeometry(w * 0.92, cabinH, suv ? len * 0.66 : len * 0.5),
    CAR_CABIN_MAT,
  );
  cabin.position.set(0, bodyH + cabinH / 2, suv ? 0 : -len * 0.06);
  g.add(body, cabin);
  return g;
}

/** A simple flat-shaded ornamental tree (trunk + low-poly canopy). */
function makeParkTree(scale: number): THREE.Object3D {
  const g = new THREE.Group();
  const trunk = new THREE.Mesh(TRUNK_GEO, TRUNK_MAT);
  const canopy = new THREE.Mesh(CANOPY_GEO, CANOPY_MAT);
  trunk.castShadow = true;
  canopy.castShadow = true;
  canopy.receiveShadow = true;
  g.add(trunk, canopy);
  g.scale.setScalar(scale);
  return g;
}

// --- Civic landmark props (procedural; only a few instances per city) ---
const CIVIC_STONE = new THREE.MeshStandardMaterial({ color: 0xe3ddcd, roughness: 0.9 });
const CIVIC_DOME = new THREE.MeshStandardMaterial({ color: 0x5b9087, roughness: 0.5, metalness: 0.3 });
const CIVIC_ROOF = new THREE.MeshStandardMaterial({ color: 0x595550, roughness: 0.9 });
const CIVIC_CLOCK = new THREE.MeshBasicMaterial({ color: 0xf4efe0 });

function cbox(w: number, h: number, d: number, mat: THREE.Material, yBase: number): THREE.Mesh {
  const m = new THREE.Mesh(new THREE.BoxGeometry(w, h, d), mat);
  m.position.y = yBase + h / 2;
  m.castShadow = true;
  m.receiveShadow = true;
  return m;
}

/** A distinctive civic landmark — a domed hall or a clocktower — so downtown
 *  has a few standout public buildings. Deterministic from the cell. */
function makeCivic(gx: number, gz: number): THREE.Object3D {
  const g = new THREE.Group();
  let v = Math.abs(Math.sin((gx * 13.7 + gz * 7.1) * 5.3));
  v -= Math.floor(v);
  if (v < 0.5) {
    // Domed hall: stylobate + body + colonnade + verdigris dome.
    const W = TILE * 0.82;
    g.add(cbox(W * 1.06, 0.3, W * 1.06, CIVIC_STONE, 0));
    g.add(cbox(W, 2.4, W, CIVIC_STONE, 0.3));
    for (let i = -2; i <= 2; i++) {
      const col = new THREE.Mesh(new THREE.CylinderGeometry(0.11, 0.11, 2.0, 8), CIVIC_STONE);
      col.position.set(i * W * 0.2, 0.3 + 1.0, W * 0.44);
      col.castShadow = true;
      g.add(col);
    }
    const dome = new THREE.Mesh(
      new THREE.SphereGeometry(W * 0.34, 14, 8, 0, Math.PI * 2, 0, Math.PI / 2),
      CIVIC_DOME,
    );
    dome.position.y = 0.3 + 2.4;
    dome.castShadow = true;
    g.add(dome);
    const finial = new THREE.Mesh(new THREE.ConeGeometry(0.1, 0.5, 8), CIVIC_DOME);
    finial.position.y = 0.3 + 2.4 + W * 0.34 + 0.1;
    g.add(finial);
  } else {
    // Clocktower: square base + tall tower + clock faces + spire.
    const B = TILE * 0.5;
    g.add(cbox(B, 2.0, B, CIVIC_STONE, 0));
    const T = TILE * 0.3;
    g.add(cbox(T, 3.4, T, CIVIC_STONE, 2.0));
    const cy = 2.0 + 2.9;
    const clockGeo = new THREE.CircleGeometry(T * 0.32, 14);
    const faces: Array<[number, number, number, number]> = [
      [0, cy, T / 2 + 0.02, 0],
      [0, cy, -T / 2 - 0.02, Math.PI],
      [T / 2 + 0.02, cy, 0, Math.PI / 2],
      [-T / 2 - 0.02, cy, 0, -Math.PI / 2],
    ];
    for (const [x, y, z, ry] of faces) {
      const clock = new THREE.Mesh(clockGeo, CIVIC_CLOCK);
      clock.position.set(x, y, z);
      clock.rotation.y = ry;
      g.add(clock);
    }
    const spire = new THREE.Mesh(new THREE.ConeGeometry(T * 0.78, 1.5, 4), CIVIC_ROOF);
    spire.position.y = 2.0 + 3.4 + 0.75;
    spire.castShadow = true;
    g.add(spire);
  }
  return g;
}

export class TileMap implements GameSystem {
  readonly name = "tiles";
  readonly group = new THREE.Group();

  private readonly roadGroup = new THREE.Group();
  private readonly lotGroup = new THREE.Group();
  private readonly buildGroup = new THREE.Group();
  private readonly parkGroup = new THREE.Group();
  private readonly carparkGroup = new THREE.Group();
  private readonly civicGroup = new THREE.Group();

  private state = new Map<string, TileState>();
  private buildings = new Map<string, BuildingView>();
  private rising = new Set<BuildingView>();
  private roadSig = "";
  private lotSig = "";
  private parkSig = "";
  private carparkSig = "";
  private civicSig = "";

  constructor(private readonly events: EventBus) {
    this.group.add(
      this.roadGroup,
      this.lotGroup,
      this.buildGroup,
      this.parkGroup,
      this.carparkGroup,
      this.civicGroup,
    );
  }

  /** Current kind at a cell (for placement validation / cursor). */
  kindAt(gx: number, gz: number): TileKind | null {
    return this.state.get(cellKey(gx, gz))?.kind ?? null;
  }

  /** Every built cell (road or zone) — vegetation keeps clear of these. */
  occupiedCells(): Set<string> {
    return new Set(this.state.keys());
  }

  isRoadAt(gx: number, gz: number): boolean {
    const k = this.state.get(cellKey(gx, gz))?.kind;
    return k === "road" || k === "avenue";
  }

  /** All road cells incl. avenues (for the traffic/people/lamp graph). */
  roadCells(): Array<{ gx: number; gz: number }> {
    const out: Array<{ gx: number; gz: number }> = [];
    for (const [key, t] of this.state) {
      if (t.kind !== "road" && t.kind !== "avenue") continue;
      const [gx, gz] = key.split(",").map(Number);
      out.push({ gx, gz });
    }
    return out;
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

    // --- Roads (incl. avenues): rebuild only if the road set changed ---
    const roadCells: Cell[] = [];
    for (const [key, t] of next) {
      if (t.kind === "road" || t.kind === "avenue") {
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
      this.rebuildRoads(roadCells);
    }

    // --- Zone lots: a bright zone tint on tiles that are zoned but not
    //     yet built. Rebuilds when a tile is zoned OR develops (the
    //     tint vanishes once a building rises). ---
    const zoneCells = [...next.entries()].filter(
      ([, t]) =>
        (t.kind === "res" || t.kind === "com" || t.kind === "ind") && t.level < 1,
    );
    const lotSig = zoneCells
      .map(([k, t]) => k + t.kind)
      .sort()
      .join("|");
    if (lotSig !== this.lotSig) {
      this.lotSig = lotSig;
      this.lotGroup.clear();
      if (zoneCells.length) this.lotGroup.add(this.buildLots(zoneCells));
    }

    // --- Parks: seeded pocket-greenspace (mowed lawn + fountain + trees),
    //     rebuilt only when the park set changes. ---
    const parkCells: Cell[] = [];
    for (const [key, t] of next) {
      if (t.kind !== "park") continue;
      const [gx, gz] = key.split(",").map(Number);
      parkCells.push({ gx, gz });
    }
    const parkSig = parkCells.map((c) => c.gx + ":" + c.gz).sort().join("|");
    if (parkSig !== this.parkSig) {
      this.parkSig = parkSig;
      this.buildParks(parkCells);
    }

    // --- Parking lots: seeded paved lots with bay lines + a few parked cars. ---
    const carparkCells: Cell[] = [];
    for (const [key, t] of next) {
      if (t.kind !== "carpark") continue;
      const [gx, gz] = key.split(",").map(Number);
      carparkCells.push({ gx, gz });
    }
    const carparkSig = carparkCells.map((c) => c.gx + ":" + c.gz).sort().join("|");
    if (carparkSig !== this.carparkSig) {
      this.carparkSig = carparkSig;
      this.buildCarparks(carparkCells);
    }

    // --- Civic landmarks: distinctive seeded public buildings. ---
    const civicCells: Cell[] = [];
    for (const [key, t] of next) {
      if (t.kind !== "civic") continue;
      const [gx, gz] = key.split(",").map(Number);
      civicCells.push({ gx, gz });
    }
    const civicSig = civicCells.map((c) => c.gx + ":" + c.gz).sort().join("|");
    if (civicSig !== this.civicSig) {
      this.civicSig = civicSig;
      this.civicGroup.clear();
      for (const { gx, gz } of civicCells) {
        const cx = cellCenterX(gx);
        const cz = cellCenterZ(gz);
        const civic = makeCivic(gx, gz);
        civic.position.set(cx, heightAt(cx, cz), cz);
        this.civicGroup.add(civic);
      }
    }

    this.syncBuildings();
  }

  /** (Re)build parking lots: an asphalt pad with white bay lines and a few
   *  parked cars. Deterministic from the cell. */
  private buildCarparks(cells: Cell[]): void {
    this.carparkGroup.clear();
    for (const { gx, gz } of cells) {
      const cx = cellCenterX(gx);
      const cz = cellCenterZ(gz);
      const base = heightAt(cx, cz);
      const lot = new THREE.Group();

      const pad = new THREE.Mesh(CARPARK_PAD_GEO, CARPARK_PAD_MAT);
      pad.position.set(cx, base + 0.05, cz);
      pad.receiveShadow = true;
      lot.add(pad);

      // Bay lines: short white bars across the lot.
      for (let i = -2; i <= 2; i++) {
        const line = new THREE.Mesh(CARPARK_LINE_GEO, CARPARK_LINE_MAT);
        line.position.set(cx + i * TILE * 0.18, base + 0.06, cz);
        lot.add(line);
      }

      // A few parked cars (deterministic count/colour/placement).
      let h = Math.abs(Math.sin((gx * 17.3 + gz * 5.1) * 11.7));
      h -= Math.floor(h);
      const n = 2 + Math.floor(h * 2); // 2-3 cars
      for (let i = 0; i < n; i++) {
        let c = Math.abs(Math.sin((gx * 3.1 + gz * 9.7 + i) * 27.3));
        c -= Math.floor(c);
        const car = makeParkedCar(CAR_COLORS[(c * CAR_COLORS.length) | 0], gx * 31 + gz * 7 + i);
        const slot = (i - (n - 1) / 2) * TILE * 0.27;
        car.position.set(cx + slot, base + 0.06, cz - TILE * 0.12);
        lot.add(car);
      }

      this.carparkGroup.add(lot);
    }
  }

  /** (Re)build the seeded parks: a manicured lawn pad, a central stone
   *  fountain, and a few ornamental trees per cell. Deterministic from the
   *  cell so every client renders the same greens. */
  private buildParks(parkCells: Cell[]): void {
    this.parkGroup.clear();
    for (const { gx, gz } of parkCells) {
      const cx = cellCenterX(gx);
      const cz = cellCenterZ(gz);
      const base = heightAt(cx, cz);
      const park = new THREE.Group();

      const lawn = new THREE.Mesh(PARK_LAWN_GEO, PARK_LAWN_MAT);
      lawn.position.set(cx, base + 0.06, cz);
      lawn.receiveShadow = true;
      park.add(lawn);

      const fountain = makeFountain();
      fountain.position.set(cx, base + 0.06, cz);
      park.add(fountain);

      // Four corner trees (deterministic slight scale jitter).
      const r = TILE * 0.3;
      const corners: Array<[number, number]> = [
        [r, r],
        [-r, r],
        [r, -r],
        [-r, -r],
      ];
      corners.forEach(([ox, oz], i) => {
        let s = Math.abs(Math.sin((gx * 13.1 + gz * 7.7 + i) * 9.13) * 41.7);
        s -= Math.floor(s);
        const tree = makeParkTree(0.85 + s * 0.4);
        tree.position.set(cx + ox, heightAt(cx + ox, cz + oz) + 0.05, cz + oz);
        tree.rotation.y = s * Math.PI * 2;
        park.add(tree);
      });

      this.parkGroup.add(park);
    }
  }

  /** Rebuild the road group: GLB tiles (auto-tiled from the neighbour
   *  mask) when the road kit is loaded, else one merged procedural mesh. */
  private rebuildRoads(roadCells: Cell[]): void {
    this.roadGroup.clear();
    if (roadCells.length === 0) return;
    const isRoad = (gx: number, gz: number) => {
      const k = this.state.get(cellKey(gx, gz))?.kind;
      return k === "road" || k === "avenue";
    };
    if (hasRoadKit()) {
      for (const { gx, gz } of roadCells) {
        const n = isRoad(gx, gz + 1);
        const e = isRoad(gx + 1, gz);
        const s = isRoad(gx, gz - 1);
        const w = isRoad(gx - 1, gz);
        const avenue = this.state.get(cellKey(gx, gz))?.kind === "avenue";
        const tile = avenue ? makeAvenueTile(n, e, s, w) : makeRoadTile(n, e, s, w);
        if (!tile) continue;
        const cx = cellCenterX(gx);
        const cz = cellCenterZ(gz);
        // Drape the tile to the ground: tilt its surface to the local terrain
        // normal so the flat asphalt follows the slope. Adjacent tiles share
        // the boundary height, so they meet instead of leaving a stepped grass
        // gap where the basin rolls (cells step up to ~0.2 m apart).
        const e2 = TILE * 0.5;
        ROAD_UP_NORMAL.set(
          heightAt(cx - e2, cz) - heightAt(cx + e2, cz),
          2 * e2,
          heightAt(cx, cz - e2) - heightAt(cx, cz + e2),
        ).normalize();
        tile.quaternion.setFromUnitVectors(ROAD_UP, ROAD_UP_NORMAL);
        tile.position.set(cx, heightAt(cx, cz) + 0.06, cz);
        this.roadGroup.add(tile);
      }
    } else {
      this.roadGroup.add(buildRoadMesh(roadCells, isRoad));
    }
  }

  /**
   * Which way a building should face. It fronts the adjacent street it lines
   * up along, not just the first road clockwise: ~half of all cells touch
   * roads on two sides (corners / through-lots), and a fixed N/E/S/W priority
   * made a row of buildings lining one avenue alternate facing the cross-
   * streets instead. So among the adjacent roads we pick the one with the
   * longest *streetwall* — the run of neighbouring buildings that front the
   * same side — biased toward avenues (a building naturally faces the bigger
   * road). Returns undefined only when no road is adjacent (kept centred +
   * randomly rotated by makeBuilding).
   */
  private frontageFaceRad(gx: number, gz: number): number | undefined {
    // [faceRad, road dir (dx,dz), streetwall axis (px,pz) perpendicular to it]
    const sides: Array<[number, number, number, number, number]> = [
      [0, 0, 1, 1, 0], // N road (+z); wall runs E-W
      [Math.PI / 2, 1, 0, 0, 1], // E road (+x); wall runs N-S
      [Math.PI, 0, -1, 1, 0], // S road (-z); wall runs E-W
      [-Math.PI / 2, -1, 0, 0, 1], // W road (-x); wall runs N-S
    ];
    const isZone = (x: number, z: number): boolean => {
      const k = this.state.get(cellKey(x, z))?.kind;
      return k === "res" || k === "com" || k === "ind";
    };
    let bestRad: number | undefined;
    let bestScore = -1;
    for (const [rad, dx, dz, px, pz] of sides) {
      const rx = gx + dx;
      const rz = gz + dz;
      if (!this.isRoadAt(rx, rz)) continue;
      // Streetwall length: this cell plus the contiguous neighbours along the
      // frontage axis that also front this same road side.
      let wall = 1;
      for (const sign of [1, -1]) {
        for (let step = 1; step < 24; step++) {
          const cx = gx + px * sign * step;
          const cz = gz + pz * sign * step;
          if (!isZone(cx, cz) || !this.isRoadAt(cx + dx, cz + dz)) break;
          wall++;
        }
      }
      const avenue = this.state.get(cellKey(rx, rz))?.kind === "avenue";
      const score = wall + (avenue ? 1.5 : 0);
      if (score > bestScore) {
        bestScore = score;
        bestRad = rad;
      }
    }
    return bestRad;
  }

  /** (Re)create building meshes to match `this.state`. Buildings whose
   *  level is unchanged are left alone. */
  private syncBuildings(): void {
    const wanted = new Set<string>();
    for (const [key, t] of this.state) {
      if (
        t.kind === "road" ||
        t.kind === "park" ||
        t.kind === "carpark" ||
        t.kind === "civic" ||
        t.level < 1
      )
        continue;
      wanted.add(key);
      const [gx, gz] = key.split(",").map(Number);
      const existing = this.buildings.get(key);
      if (!existing || existing.level !== t.level) {
        if (existing) this.buildGroup.remove(existing.obj);
        const faceRad = this.frontageFaceRad(gx, gz);
        const obj = makeBuilding(t.kind as BuildingZone, t.level, gx, gz, faceRad);
        const cx = cellCenterX(gx);
        const cz = cellCenterZ(gz);
        obj.position.set(cx, heightAt(cx, cz), cz);
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

  /** Rebuild buildings AND roads from scratch — called once the GLB kit
   *  finishes loading so the procedural placeholders are replaced. */
  refreshKit(): void {
    for (const view of this.buildings.values()) this.buildGroup.remove(view.obj);
    this.buildings.clear();
    this.rising.clear();
    this.syncBuildings();
    // Force the roads to rebuild with GLB tiles.
    const roadCells: Cell[] = [];
    for (const [key, t] of this.state) {
      if (t.kind === "road" || t.kind === "avenue") {
        const [gx, gz] = key.split(",").map(Number);
        roadCells.push({ gx, gz });
      }
    }
    this.rebuildRoads(roadCells);
  }

  private buildLots(cells: Array<[string, TileState]>): THREE.Mesh {
    // Bright zoning colours (SimCity/CS style): green / blue / amber.
    const TINT: Record<BuildingZone, [number, number, number]> = {
      res: [0.35, 0.82, 0.35],
      com: [0.33, 0.72, 0.9],
      ind: [0.9, 0.72, 0.24],
    };
    const pos: number[] = [];
    const col: number[] = [];
    for (const [key, t] of cells) {
      if (t.kind === "road") continue;
      const [gx, gz] = key.split(",").map(Number);
      const cx = cellCenterX(gx);
      const cz = cellCenterZ(gz);
      const c = TINT[t.kind as BuildingZone];
      const s = TILE * 0.48;
      // Conform to the terrain (sample each corner) and lift clear so the
      // undulating mesh doesn't bury the flat overlay.
      const L = 0.25;
      const y00 = heightAt(cx - s, cz - s) + L;
      const y10 = heightAt(cx + s, cz - s) + L;
      const y11 = heightAt(cx + s, cz + s) + L;
      const y01 = heightAt(cx - s, cz + s) + L;
      const v = [
        cx - s, y00, cz - s, cx + s, y10, cz - s, cx + s, y11, cz + s,
        cx - s, y00, cz - s, cx + s, y11, cz + s, cx - s, y01, cz + s,
      ];
      pos.push(...v);
      for (let i = 0; i < 6; i++) col.push(c[0], c[1], c[2]);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    geo.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
    // Unlit + translucent so it reads as a planning overlay on the grass.
    const mesh = new THREE.Mesh(
      geo,
      new THREE.MeshBasicMaterial({
        vertexColors: true,
        transparent: true,
        opacity: 0.6,
        depthWrite: false,
        side: THREE.DoubleSide,
      }),
    );
    mesh.renderOrder = 1;
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
