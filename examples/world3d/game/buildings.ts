/**
 * Destructible block buildings.
 *
 * A handful of compounds are generated deterministically on flat
 * ground: hollow shells of BLOCK_SIZE cubes with windows, doorways,
 * floor slabs, and roofs. Every block lives in ONE InstancedMesh
 * (single draw call); destroyed blocks are hidden with a zero-scale
 * matrix so instance ids stay stable for raycasts.
 *
 * Destruction is structural: after blocks are removed, a flood fill
 * from the ground layer finds every block no longer connected to
 * ground and detaches it too. Collapse is a pure function of the
 * destroyed-key set, so multiplayer only syncs directly-hit keys —
 * each client derives identical rubble.
 *
 * Block key format: "<building>:<gx>:<gy>:<gz>" (grid coords).
 */
import * as THREE from "three";
import { BLOCK_SIZE, SEED, WATER_LEVEL, WORLD_HALF } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import { makeRng, type Rng } from "./prng";
import type { Terrain } from "./terrain";

interface Block {
  instanceId: number;
  building: number;
  gx: number;
  gy: number;
  gz: number;
  color: THREE.Color;
  alive: boolean;
}

interface Building {
  origin: THREE.Vector3; // world position of grid (0, 0, 0) block center
  w: number; // blocks along x
  h: number; // blocks along y
  d: number; // blocks along z
  /** local "gx,gy,gz" → block */
  grid: Map<string, Block>;
}

export interface BlockHit {
  key: string;
  center: THREE.Vector3;
}

export class Buildings implements GameSystem {
  readonly name = "buildings";
  readonly mesh: THREE.InstancedMesh;

  private readonly buildings: Building[] = [];
  private readonly byInstance: Block[] = [];
  private readonly byKey = new Map<string, Block>();
  private readonly destroyed = new Set<string>();

  private readonly hidden = new THREE.Matrix4().makeScale(0, 0, 0);
  private readonly tmpM = new THREE.Matrix4();
  private readonly raycaster = new THREE.Raycaster();

  /**
   * Plan compound sites: score candidates by footprint relief and
   * greedily keep the flattest, mutually-distant ones. No hard
   * flatness gate — the terrain gets terraformed into a pad under
   * each site (terrain.flattenArea), so "flat enough" always exists.
   * (The old hard 1.6 m gate found ZERO sites on this island; the HUD
   * read "ruins 0/0" and nobody noticed until a playtest asked where
   * the compounds were.)
   */
  static planSites(terrain: Terrain, want: number): THREE.Vector3[] {
    const rng = makeRng(SEED ^ 0xb01d);
    interface Candidate {
      x: number;
      z: number;
      relief: number;
      h: number;
    }
    const candidates: Candidate[] = [];
    for (let i = 0; i < 900; i++) {
      const x = rng.range(-WORLD_HALF * 0.6, WORLD_HALF * 0.6);
      const z = rng.range(-WORLD_HALF * 0.6, WORLD_HALF * 0.6);
      const h = terrain.heightAt(x, z);
      if (h < WATER_LEVEL + 1.5 || h > WATER_LEVEL + 20) continue;
      let minH = Infinity;
      let maxH = -Infinity;
      for (let dx = -8; dx <= 8; dx += 4) {
        for (let dz = -8; dz <= 8; dz += 4) {
          const s = terrain.heightAt(x + dx, z + dz);
          minH = Math.min(minH, s);
          maxH = Math.max(maxH, s);
        }
      }
      const relief = maxH - minH;
      if (relief > 6) continue; // don't carve pads into cliff faces
      candidates.push({ x, z, relief, h });
    }
    candidates.sort((a, b) => a.relief - b.relief);

    const sites: THREE.Vector3[] = [];
    for (const c of candidates) {
      if (sites.length >= want) break;
      if (sites.some((s) => Math.hypot(s.x - c.x, s.z - c.z) < 48)) continue;
      sites.push(new THREE.Vector3(c.x, c.h, c.z));
    }
    return sites;
  }

  constructor(
    private readonly terrain: Terrain,
    /** Pre-terraformed pad sites (y = pad elevation from flattenArea). */
    sites: THREE.Vector3[],
    concreteMap?: THREE.Texture,
    concreteNormal?: THREE.Texture,
  ) {
    const rng = makeRng(SEED ^ 0xb02e);
    for (const site of sites) this.generateBuilding(rng, site);

    const total = this.byInstance.length;
    const geo = new THREE.BoxGeometry(BLOCK_SIZE, BLOCK_SIZE, BLOCK_SIZE);
    const mat = new THREE.MeshStandardMaterial({
      roughness: 0.85,
      metalness: 0.02,
      map: concreteMap ?? null,
      normalMap: concreteNormal ?? null,
    });
    this.mesh = new THREE.InstancedMesh(geo, mat, Math.max(1, total));
    this.mesh.instanceColor = new THREE.InstancedBufferAttribute(new Float32Array(total * 3), 3);
    this.mesh.castShadow = true;
    this.mesh.receiveShadow = true;
    this.mesh.name = "buildings";

    for (const block of this.byInstance) {
      this.tmpM.makeTranslation(...this.blockCenter(block).toArray());
      this.mesh.setMatrixAt(block.instanceId, this.tmpM);
      this.mesh.instanceColor.setXYZ(block.instanceId, block.color.r, block.color.g, block.color.b);
    }
    this.mesh.instanceMatrix.needsUpdate = true;
    this.mesh.instanceColor.needsUpdate = true;
  }

  private generateBuilding(rng: Rng, site: THREE.Vector3) {
    const index = this.buildings.length;
    const w = rng.int(5) + 7; // 7–11 blocks
    const d = rng.int(5) + 6; // 6–10 blocks
    const stories = rng.next() < 0.4 ? 2 : 1;
    const storyH = 4;
    const h = stories * storyH + 1; // +1 roof layer

    const wallTints = ["#c8c2b4", "#b9c0c6", "#cdb9a4", "#aab4ac"] as const;
    const wall = new THREE.Color(rng.pick(wallTints));
    const trim = wall.clone().multiplyScalar(0.72);
    const roof = new THREE.Color("#7c6f5e").multiplyScalar(rng.range(0.85, 1.1));

    const building: Building = {
      origin: new THREE.Vector3(
        site.x - ((w - 1) * BLOCK_SIZE) / 2,
        site.y + BLOCK_SIZE * 0.5 - 0.3, // sink slightly into the ground
        site.z - ((d - 1) * BLOCK_SIZE) / 2,
      ),
      w,
      h,
      d,
      grid: new Map(),
    };
    this.buildings.push(building);

    const doorWall = rng.int(4); // which side gets the doorway
    const doorAt = 1 + rng.int(Math.max(1, (doorWall < 2 ? w : d) - 3));

    const place = (gx: number, gy: number, gz: number, color: THREE.Color) => {
      const block: Block = {
        instanceId: this.byInstance.length,
        building: index,
        gx,
        gy,
        gz,
        color,
        alive: true,
      };
      this.byInstance.push(block);
      building.grid.set(`${gx},${gy},${gz}`, block);
      this.byKey.set(this.keyOf(block), block);
    };

    for (let gy = 0; gy < h; gy++) {
      const isRoof = gy === h - 1;
      const isFloorSlab = stories === 2 && gy === storyH;
      for (let gz = 0; gz < d; gz++) {
        for (let gx = 0; gx < w; gx++) {
          const isShell = gx === 0 || gx === w - 1 || gz === 0 || gz === d - 1;
          if (isRoof || isFloorSlab) {
            place(gx, gy, gz, isRoof ? roof : trim);
            continue;
          }
          if (!isShell) continue; // hollow interior

          // Doorway: 2-block gap on the chosen wall, ground story only.
          if (gy < 3) {
            const onDoorWall =
              (doorWall === 0 && gz === 0) ||
              (doorWall === 1 && gz === d - 1) ||
              (doorWall === 2 && gx === 0) ||
              (doorWall === 3 && gx === w - 1);
            const along = doorWall < 2 ? gx : gz;
            if (onDoorWall && (along === doorAt || along === doorAt + 1)) continue;
          }

          // Windows: every other column, mid-height of each story.
          const storyY = gy % storyH;
          const along = gz === 0 || gz === d - 1 ? gx : gz;
          const isWindow =
            storyY === 2 && along % 2 === 1 && along > 0 && along < (gz === 0 || gz === d - 1 ? w : d) - 1;
          if (isWindow) continue;

          const isCorner = (gx === 0 || gx === w - 1) && (gz === 0 || gz === d - 1);
          place(gx, gy, gz, isCorner || gy === 0 ? trim : wall);
        }
      }
    }
  }

  private keyOf(block: Block): string {
    return `${block.building}:${block.gx}:${block.gy}:${block.gz}`;
  }

  blockCenter(block: Block, out = new THREE.Vector3()): THREE.Vector3 {
    const b = this.buildings[block.building];
    return out.set(
      b.origin.x + block.gx * BLOCK_SIZE,
      b.origin.y + block.gy * BLOCK_SIZE,
      b.origin.z + block.gz * BLOCK_SIZE,
    );
  }

  colorOf(key: string): THREE.Color {
    return this.byKey.get(key)?.color ?? new THREE.Color("#b0a89a");
  }

  /** Total / destroyed counts for the HUD. */
  get stats(): { total: number; destroyed: number } {
    return { total: this.byInstance.length, destroyed: this.destroyed.size };
  }

  /** Compound centers + footprint size in meters, for the minimap. */
  siteFootprints(): Array<{ x: number; z: number; size: number }> {
    return this.buildings.map((b) => ({
      x: b.origin.x + ((b.w - 1) * BLOCK_SIZE) / 2,
      z: b.origin.z + ((b.d - 1) * BLOCK_SIZE) / 2,
      size: Math.max(b.w, b.d) * BLOCK_SIZE,
    }));
  }

  /** Raycast from camera/projectile against live blocks. */
  raycastBlocks(origin: THREE.Vector3, direction: THREE.Vector3, far: number): BlockHit | null {
    this.raycaster.set(origin, direction);
    this.raycaster.far = far;
    const hits = this.raycaster.intersectObject(this.mesh, false);
    for (const hit of hits) {
      if (hit.instanceId === undefined) continue;
      const block = this.byInstance[hit.instanceId];
      if (!block?.alive) continue; // zero-scale instances shouldn't hit, but be safe
      return { key: this.keyOf(block), center: this.blockCenter(block) };
    }
    return null;
  }

  /** Distance the ray travels before hitting a block (for tracer endpoints). */
  raycastDistance(origin: THREE.Vector3, direction: THREE.Vector3, far: number): number | null {
    const hit = this.raycastBlocks(origin, direction, far);
    if (!hit) return null;
    return hit.center.distanceTo(origin);
  }

  /** All live block keys within `radius` of a point (explosions). */
  keysInRadius(center: THREE.Vector3, radius: number): string[] {
    const keys: string[] = [];
    const tmp = new THREE.Vector3();
    const r2 = radius * radius;
    // Coarse building-level cull first, then exact block distances.
    for (let bi = 0; bi < this.buildings.length; bi++) {
      const b = this.buildings[bi];
      const half = (Math.max(b.w, b.h, b.d) * BLOCK_SIZE) / 2;
      tmp.set(
        b.origin.x + ((b.w - 1) * BLOCK_SIZE) / 2,
        b.origin.y + ((b.h - 1) * BLOCK_SIZE) / 2,
        b.origin.z + ((b.d - 1) * BLOCK_SIZE) / 2,
      );
      if (tmp.distanceTo(center) > radius + half * 1.8) continue;
      for (const block of b.grid.values()) {
        if (!block.alive) continue;
        if (this.blockCenter(block, tmp).distanceToSquared(center) <= r2) {
          keys.push(this.keyOf(block));
        }
      }
    }
    return keys;
  }

  /**
   * Destroy blocks by key and run structural collapse. Returns every
   * removed block (direct + collapsed) so callers can spawn debris.
   * Unknown/already-destroyed keys are ignored — remote rows replay
   * safely and out of order.
   */
  destroyKeys(keys: string[]): BlockHit[] {
    const removed: BlockHit[] = [];
    const touchedBuildings = new Set<number>();

    for (const key of keys) {
      const block = this.byKey.get(key);
      if (!block || !block.alive) continue;
      this.removeBlock(block, removed);
      touchedBuildings.add(block.building);
    }

    // Structural pass per touched building.
    for (const bi of touchedBuildings) {
      for (const block of this.collapseUnsupported(bi)) {
        this.removeBlock(block, removed);
      }
    }

    if (removed.length > 0) this.mesh.instanceMatrix.needsUpdate = true;
    return removed;
  }

  private removeBlock(block: Block, removed: BlockHit[]) {
    block.alive = false;
    this.destroyed.add(this.keyOf(block));
    this.mesh.setMatrixAt(block.instanceId, this.hidden);
    removed.push({ key: this.keyOf(block), center: this.blockCenter(block) });
  }

  /** Blocks no longer connected to the ground layer via 6-adjacency. */
  private collapseUnsupported(buildingIndex: number): Block[] {
    const b = this.buildings[buildingIndex];
    const reached = new Set<Block>();
    const queue: Block[] = [];

    for (const block of b.grid.values()) {
      if (block.alive && block.gy === 0) {
        reached.add(block);
        queue.push(block);
      }
    }

    const NEIGHBORS = [
      [1, 0, 0], [-1, 0, 0],
      [0, 1, 0], [0, -1, 0],
      [0, 0, 1], [0, 0, -1],
    ] as const;

    while (queue.length > 0) {
      const cur = queue.pop()!;
      for (const [dx, dy, dz] of NEIGHBORS) {
        const next = b.grid.get(`${cur.gx + dx},${cur.gy + dy},${cur.gz + dz}`);
        if (next && next.alive && !reached.has(next)) {
          reached.add(next);
          queue.push(next);
        }
      }
    }

    const orphans: Block[] = [];
    for (const block of b.grid.values()) {
      if (block.alive && !reached.has(block)) orphans.push(block);
    }
    return orphans;
  }

  /**
   * Reconcile with the synced Destruction rows. Handles both new
   * remote destruction (keys we haven't applied) and a world reset
   * (rows deleted → restore everything). Returns newly removed blocks
   * so the caller can decide whether to play effects.
   */
  syncDestroyedKeys(keys: ReadonlySet<string>): BlockHit[] {
    // Reset path: server has fewer destroyed keys than us → rebuild.
    // (Individual rows are append-only; deletion only happens en masse
    // via resetIsland, so "any of ours missing" means reset.)
    let needsReset = false;
    for (const mine of this.destroyed) {
      if (!keys.has(mine)) {
        needsReset = true;
        break;
      }
    }
    if (needsReset) this.restoreAll();

    const fresh: string[] = [];
    for (const key of keys) {
      if (!this.destroyed.has(key)) fresh.push(key);
    }
    if (fresh.length === 0) return [];
    return this.destroyKeys(fresh);
  }

  private restoreAll() {
    for (const block of this.byInstance) {
      if (block.alive) continue;
      block.alive = true;
      this.tmpM.makeTranslation(...this.blockCenter(block).toArray());
      this.mesh.setMatrixAt(block.instanceId, this.tmpM);
    }
    this.destroyed.clear();
    this.mesh.instanceMatrix.needsUpdate = true;
  }

  update(_ctx: FrameCtx) {
    // Static unless destruction happens; nothing per-frame.
  }

  dispose() {
    this.mesh.geometry.dispose();
    (this.mesh.material as THREE.Material).dispose();
    this.mesh.dispose();
  }
}
