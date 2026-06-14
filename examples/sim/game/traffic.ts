/**
 * Traffic — cars driving the road network, the cheapest big win for a
 * "living city". Local + cosmetic (not synced): each client runs its own
 * cars over the shared road graph.
 *
 * Cars random-walk the graph of road cells (no backtracking unless it's a
 * dead end), interpolating cell-to-cell on the right-hand lane, draped on
 * the terrain and pointed along travel. The fleet size tracks the amount
 * of road.
 */
import * as THREE from "three";
import { DRACOLoader } from "three/examples/jsm/loaders/DRACOLoader.js";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { TILE } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import { cellCenterX, cellCenterZ, cellKey } from "./grid";
import { heightAt } from "./terrain";

const CAR_FILES = [
  "NormalCar1.glb",
  "NormalCar2.glb",
  "SUV.glb",
  "Taxi.glb",
  "Cop.glb",
  "SportsCar.glb",
  "SportsCar2.glb",
];

const CAR_LENGTH = 2.6; // metres along travel
const LANE = TILE * 0.15; // right-of-centre offset

/** Colour the named car materials (the pack ships them grey). */
const CAR_PALETTE: Array<[RegExp, number]> = [
  [/taillight/i, 0xd02020],
  [/bluelight/i, 0x2a5cf0],
  [/whitelight|headlight/i, 0xfff3cf],
  [/window/i, 0x223342],
  [/black/i, 0x191b1e],
  [/darkgrey|darkgray/i, 0x3a3d42],
  [/grey|gray/i, 0x8b9097],
  [/blue/i, 0x2f63b8],
  [/yellow/i, 0xe7bf2a],
  [/red/i, 0xb83434],
  [/green/i, 0x2f8a52],
  [/white/i, 0xe2e4e6],
];
function carColor(name: string): number | null {
  for (const [re, hex] of CAR_PALETTE) if (re.test(name)) return hex;
  return null;
}

const templates: THREE.Object3D[] = [];

export async function preloadCars(basePath = "/models/citykit/cars/"): Promise<void> {
  if (templates.length) return;
  const draco = new DRACOLoader().setDecoderPath(
    "https://www.gstatic.com/draco/versioned/decoders/1.5.7/",
  );
  const loader = new GLTFLoader().setDRACOLoader(draco);
  await Promise.all(
    CAR_FILES.map(
      (file) =>
        new Promise<void>((resolve) => {
          loader.load(
            basePath + file,
            (gltf) => {
              templates.push(normalizeCar(gltf.scene));
              resolve();
            },
            undefined,
            () => resolve(),
          );
        }),
    ),
  );
}

/**
 * Scale the car to ~CAR_LENGTH, recentre in x/z, drop its base to y=0,
 * and colour the named materials. (GLTFLoader already applies assimp's
 * Z-up→Y-up node transform, so the loaded car is already lying flat — no
 * extra rotation.) Forward is the model's local +Z.
 */
function normalizeCar(scene: THREE.Object3D): THREE.Object3D {
  scene.updateWorldMatrix(true, true);
  const box = new THREE.Box3().setFromObject(scene);
  const size = new THREE.Vector3();
  const center = new THREE.Vector3();
  box.getSize(size);
  box.getCenter(center);
  const span = Math.max(size.x, size.z) || 1;
  const scale = CAR_LENGTH / span;

  scene.traverse((o) => {
    const m = o as THREE.Mesh;
    if (!m.isMesh) return;
    m.castShadow = true;
    const mats = Array.isArray(m.material) ? m.material : [m.material];
    for (const mat of mats) {
      const std = mat as THREE.MeshStandardMaterial;
      if (!std.isMeshStandardMaterial) continue;
      const hex = carColor(std.name);
      if (hex !== null) std.color.setHex(hex);
      std.metalness = 0.1;
      std.roughness = 0.55;
      std.flatShading = true;
      std.needsUpdate = true;
    }
  });

  scene.position.set(-center.x, -box.min.y, -center.z);
  const holder = new THREE.Group();
  holder.add(scene);
  holder.scale.setScalar(scale);
  return holder;
}

interface Car {
  obj: THREE.Object3D;
  from: string;
  to: string;
  t: number;
  speed: number;
}

export class Traffic implements GameSystem {
  readonly name = "traffic";
  readonly group = new THREE.Group();

  private graph = new Map<string, string[]>();
  private cars: Car[] = [];
  private roadSig = "";
  private readonly tmpA = new THREE.Vector3();
  private readonly tmpB = new THREE.Vector3();

  /** Rebuild the road graph; resize the fleet. Cheap no-op if unchanged. */
  setRoads(roadCells: Array<{ gx: number; gz: number }>, isRoad: (gx: number, gz: number) => boolean): void {
    const sig = roadCells.map((c) => c.gx + ":" + c.gz).sort().join("|");
    if (sig !== this.roadSig) {
      this.roadSig = sig;
      this.graph.clear();
      for (const { gx, gz } of roadCells) {
        const nb: string[] = [];
        if (isRoad(gx + 1, gz)) nb.push(cellKey(gx + 1, gz));
        if (isRoad(gx - 1, gz)) nb.push(cellKey(gx - 1, gz));
        if (isRoad(gx, gz + 1)) nb.push(cellKey(gx, gz + 1));
        if (isRoad(gx, gz - 1)) nb.push(cellKey(gx, gz - 1));
        if (nb.length) this.graph.set(cellKey(gx, gz), nb);
      }
    }

    // Always re-size the fleet (cars may finish loading AFTER the graph
    // was first built, so this can't sit behind the sig guard).
    if (templates.length === 0) return;
    const target = Math.min(60, Math.floor(this.graph.size / 2.5));
    while (this.cars.length > target) {
      const c = this.cars.pop()!;
      this.group.remove(c.obj);
    }
    while (this.cars.length < target) {
      const car = this.spawn();
      if (!car) break;
      this.cars.push(car);
      this.group.add(car.obj);
    }
    // Re-home cars whose road was bulldozed out from under them.
    for (const car of this.cars) {
      if (!this.graph.has(car.from) || !this.graph.has(car.to)) {
        const fresh = this.spawn();
        if (fresh) {
          car.from = fresh.from;
          car.to = fresh.to;
          car.t = fresh.t;
          this.group.remove(fresh.obj);
        }
      }
    }
  }

  private spawn(): Car | null {
    const keys = [...this.graph.keys()];
    if (keys.length === 0) return null;
    const from = keys[(Math.random() * keys.length) | 0];
    const nb = this.graph.get(from)!;
    const to = nb[(Math.random() * nb.length) | 0];
    const obj = templates[(Math.random() * templates.length) | 0].clone(true);
    return { obj, from, to, t: Math.random(), speed: 7 + Math.random() * 5 };
  }

  private next(from: string, to: string): string {
    const nb = this.graph.get(to);
    if (!nb || nb.length === 0) return from;
    const fwd = nb.filter((k) => k !== from);
    const pool = fwd.length ? fwd : nb; // dead end → U-turn
    return pool[(Math.random() * pool.length) | 0];
  }

  update(ctx: FrameCtx): void {
    if (this.cars.length === 0) return;
    for (const car of this.cars) {
      car.t += (car.speed * ctx.dt) / TILE;
      while (car.t >= 1) {
        car.t -= 1;
        const nf = car.to;
        car.to = this.next(car.from, car.to);
        car.from = nf;
      }
      const [fgx, fgz] = car.from.split(",").map(Number);
      const [tgx, tgz] = car.to.split(",").map(Number);
      this.tmpA.set(cellCenterX(fgx), 0, cellCenterZ(fgz));
      this.tmpB.set(cellCenterX(tgx), 0, cellCenterZ(tgz));
      const dx = this.tmpB.x - this.tmpA.x;
      const dz = this.tmpB.z - this.tmpA.z;
      const len = Math.hypot(dx, dz) || 1;
      const ux = dx / len;
      const uz = dz / len;
      // Right-hand lane: offset to the right of travel.
      const rx = uz;
      const rz = -ux;
      const px = this.tmpA.x + dx * car.t + rx * LANE;
      const pz = this.tmpA.z + dz * car.t + rz * LANE;
      car.obj.position.set(px, heightAt(px, pz) + 0.12, pz);
      car.obj.rotation.y = Math.atan2(ux, uz);
    }
  }

  dispose(): void {
    this.group.clear();
    this.cars = [];
  }
}
