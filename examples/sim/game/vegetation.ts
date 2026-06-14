/**
 * Vegetation — the lush layer that makes a city read as a place rather
 * than blocks on a plane (the Cities-Skylines look is mostly trees).
 *
 * Loads the low-poly tree GLBs, colours their named materials (the pack
 * ships them grey — colour lived in a texture atlas we don't have),
 * then scatters thousands of them with InstancedMesh: forest clumps
 * from value noise across open land, plus parkway trees lining the
 * streets. Everything is deterministic from the cell coordinate, so
 * every client renders the same forest. Trees never cover a road, zone
 * or building.
 */
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { GRID, TILE, WATER_LEVEL } from "./config";
import type { GameSystem } from "./engine";
import { cellCenterX, cellCenterZ, cellKey } from "./grid";
import { hash2 } from "./prng";
import { heightAt, slopeAt } from "./terrain";

const TREE_FILES = [
  "Tree_1.glb",
  "Tree_2.glb",
  "Tree_3.glb",
  "Birch_1.glb",
  "Birch_3.glb",
  "Birch_5.glb",
  "Pine_1.glb",
];

/** A tree template = its parts (bark / leaves) with coloured materials. */
interface TreePart {
  geometry: THREE.BufferGeometry;
  material: THREE.Material;
  isLeaves: boolean;
  /** Natural leaf colour (leaf materials are white so the per-instance
   *  colour — this or a blossom tint — shows true rather than muddied). */
  leafBase?: THREE.Color;
}
interface TreeTemplate {
  parts: TreePart[];
  /** World height (m) the model was normalised to. */
  height: number;
}

const templates: TreeTemplate[] = [];

function leafColor(name: string): THREE.Color {
  if (/pine/i.test(name)) return new THREE.Color(0x2f5a32);
  if (/birch/i.test(name)) return new THREE.Color(0x6fa64a);
  return new THREE.Color(0x4f8438);
}
function barkColor(name: string): THREE.Color {
  if (/birch/i.test(name)) return new THREE.Color(0xcdc8ba);
  return new THREE.Color(0x5a4630);
}

/** Load + colour + normalise the tree set. Safe to call once. */
export async function preloadTrees(basePath = "/models/citykit/trees/"): Promise<void> {
  if (templates.length) return;
  const loader = new GLTFLoader();
  await Promise.all(
    TREE_FILES.map(
      (file) =>
        new Promise<void>((resolve) => {
          loader.load(
            basePath + file,
            (gltf) => {
              const tpl = buildTemplate(gltf.scene);
              if (tpl) templates.push(tpl);
              resolve();
            },
            undefined,
            () => resolve(),
          );
        }),
    ),
  );
}

function buildTemplate(scene: THREE.Object3D): TreeTemplate | null {
  scene.updateWorldMatrix(true, true);
  const box = new THREE.Box3().setFromObject(scene);
  const size = new THREE.Vector3();
  box.getSize(size);
  const center = new THREE.Vector3();
  box.getCenter(center);
  const nativeH = size.y || 1;
  const targetH = 6 + (size.y > 0 ? 0 : 0); // set per-instance scale later
  const scale = 1 / nativeH; // normalise to height 1; instances scale up
  // Bake recentre (base y=0, centred) + normalise into each geometry.
  const m = new THREE.Matrix4()
    .makeScale(scale, scale, scale)
    .multiply(new THREE.Matrix4().makeTranslation(-center.x, -box.min.y, -center.z));

  const parts: TreePart[] = [];
  scene.traverse((o) => {
    const mesh = o as THREE.Mesh;
    if (!mesh.isMesh || !mesh.geometry) return;
    mesh.updateWorldMatrix(true, false);
    const geo = mesh.geometry.clone();
    geo.applyMatrix4(m.clone().multiply(mesh.matrixWorld));
    const srcMat = (Array.isArray(mesh.material) ? mesh.material[0] : mesh.material) as THREE.Material;
    const name = srcMat?.name ?? "";
    const isLeaves = /leaf|leaves/i.test(name);
    const material = new THREE.MeshStandardMaterial({
      // Leaves: white so the per-instance colour shows true. Bark keeps
      // its own colour (no per-instance colour).
      color: isLeaves ? 0xffffff : barkColor(name),
      roughness: 1,
      metalness: 0,
      flatShading: true,
    });
    parts.push({
      geometry: geo,
      material,
      isLeaves,
      leafBase: isLeaves ? leafColor(name) : undefined,
    });
  });
  if (parts.length === 0) return null;
  return { parts, height: targetH };
}

/** Value-noise forest density at a cell, [0,1] with smooth clumps. */
function forestDensity(gx: number, gz: number): number {
  const s = 7;
  const x0 = Math.floor(gx / s);
  const z0 = Math.floor(gz / s);
  const fx = gx / s - x0;
  const fz = gz / s - z0;
  const lerp = (a: number, b: number, t: number) => a + (b - a) * (t * t * (3 - 2 * t));
  const n = (a: number, b: number) => hash2(a, b, 0x1234);
  const top = lerp(n(x0, z0), n(x0 + 1, z0), fx);
  const bot = lerp(n(x0, z0 + 1), n(x0 + 1, z0 + 1), fx);
  return lerp(top, bot, fz);
}

// Occasional flowering / autumn trees — a restrained colour pop, not the
// candy-pink carpet. Muted blush + cream + autumn so the canopy stays
// believably green-dominant like a real CS suburb.
const BLOSSOM = [0xe3b6c6, 0xe7c8d2, 0xdcc8a6, 0xd2ad84, 0xc9a85a, 0xc28d6a];

export class Vegetation implements GameSystem {
  readonly name = "vegetation";
  readonly group = new THREE.Group();
  private occSig = "";
  private readonly disposables: Array<THREE.BufferGeometry | THREE.Material> = [];

  /** Rebuild the forest given the currently occupied (built) cells and
   *  which cells are roads (for parkway trees). */
  scatter(occupied: Set<string>, isRoad: (gx: number, gz: number) => boolean): void {
    if (templates.length === 0) return;
    const sig = occupied.size + ":" + [...occupied].sort().join(",");
    if (sig === this.occSig) return;
    this.occSig = sig;
    this.clear();

    // Collect per-(template, part) instance matrices + leaf colours.
    const mats: THREE.Matrix4[][][] = templates.map((t) => t.parts.map(() => []));
    const cols: (THREE.Color | null)[][][] = templates.map((t) => t.parts.map(() => []));
    const dummy = new THREE.Object3D();
    const tmpC = new THREE.Color();

    const place = (wx: number, wz: number, seed: number) => {
      // No trees in water or on cliffs.
      const ground = heightAt(wx, wz);
      if (ground < WATER_LEVEL + 0.4 || slopeAt(wx, wz) > 1.3 || ground > 90) return;
      const ti = Math.floor(hash2(seed, 1, 9) * templates.length) % templates.length;
      const tpl = templates[ti];
      const h = 3 + hash2(seed, 2, 9) * 3.8; // 3–6.8 m
      const sxz = h * (0.5 + hash2(seed, 5, 9) * 0.25);
      dummy.position.set(wx, ground, wz);
      dummy.rotation.set(0, hash2(seed, 3, 9) * Math.PI * 2, 0);
      dummy.scale.set(sxz, h, sxz);
      dummy.updateMatrix();
      const blossom = hash2(seed, 7, 9) < 0.07;
      tpl.parts.forEach((p, pi) => {
        mats[ti][pi].push(dummy.matrix.clone());
        // Leaf instances always carry a colour (blossom tint or the
        // template's natural green); bark carries none (shows its material).
        if (!p.isLeaves) {
          cols[ti][pi].push(null);
        } else if (blossom) {
          cols[ti][pi].push(tmpC.setHex(BLOSSOM[Math.floor(hash2(seed, 8, 9) * BLOSSOM.length)]).clone());
        } else {
          cols[ti][pi].push((p.leafBase ?? tmpC.setHex(0x4f8438)).clone());
        }
      });
    };

    // Dense scatter: most open land gets trees (forest noise sets the
    // count), every cell beside a road gets extra yard/street trees. This
    // carpets the map and fills the gaps between buildings — the lush CS
    // look — rather than leaving bare grass.
    let total = 0;
    const MAX = 18000;
    for (let gx = 0; gx < GRID && total < MAX; gx++) {
      for (let gz = 0; gz < GRID && total < MAX; gz++) {
        if (occupied.has(cellKey(gx, gz))) continue;
        const cx = cellCenterX(gx);
        const cz = cellCenterZ(gz);
        const d = forestDensity(gx, gz);
        // Thick away from the city (real forest), a yard tree or two near
        // the streets — but never so dense it buries the houses.
        let count = d > 0.78 ? 2 : d > 0.5 ? 1 : 0;
        const nextToRoad =
          isRoad(gx + 1, gz) || isRoad(gx - 1, gz) || isRoad(gx, gz + 1) || isRoad(gx, gz - 1);
        if (nextToRoad) count = Math.max(count, hash2(gx, gz, 31) < 0.6 ? 1 : 0);
        for (let k = 0; k < count && total < MAX; k++) {
          const seed = (gx * 131 + gz) * 7 + k;
          place(
            cx + (hash2(gx, gz + k * 3, 21) - 0.5) * TILE * 0.6,
            cz + (hash2(gx + k * 3, gz, 22) - 0.5) * TILE * 0.6,
            seed,
          );
          total++;
        }
      }
    }

    // Build one InstancedMesh per (template, part).
    templates.forEach((tpl, ti) => {
      tpl.parts.forEach((part, pi) => {
        const list = mats[ti][pi];
        if (list.length === 0) return;
        const im = new THREE.InstancedMesh(part.geometry, part.material, list.length);
        im.castShadow = true;
        im.receiveShadow = true;
        for (let i = 0; i < list.length; i++) {
          im.setMatrixAt(i, list[i]);
          const c = cols[ti][pi][i];
          if (c) im.setColorAt(i, c);
        }
        im.instanceMatrix.needsUpdate = true;
        if (im.instanceColor) im.instanceColor.needsUpdate = true;
        this.group.add(im);
      });
    });
  }

  private clear(): void {
    for (const child of this.group.children) {
      const im = child as THREE.InstancedMesh;
      im.dispose?.();
    }
    this.group.clear();
  }

  update(): void {
    /* static between scatters */
  }

  dispose(): void {
    this.clear();
    for (const t of templates) for (const p of t.parts) {
      p.geometry.dispose();
      (p.material as THREE.Material).dispose();
    }
  }
}
