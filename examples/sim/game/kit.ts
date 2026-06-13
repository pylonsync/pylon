/**
 * Building kit: turns a (zone, level) into a three.js Object3D.
 *
 * Uses Quaternius Downtown City MegaKit GLB models when present in
 * /models/citykit/, and falls back to clean procedural massing when a
 * model is missing — so the example renders a real city out of the box
 * and gets prettier the moment the asset pack is dropped in. Either
 * way a building's base sits at y=0, centred, scaled to the cell.
 */
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { TILE } from "./config";
import { ZONE } from "./config";
import { hash2 } from "./prng";

export type BuildingZone = "res" | "com" | "ind";

/**
 * Uniform scale applied to every kit model (preserves the pack's real
 * proportions, so bigger structures stay bigger across levels). Tuned so
 * the widest MegaKit building (~21 m) lands at roughly one cell.
 */
const KIT_SCALE = TILE * 0.052;

/** Footprint for the procedural fallback massing. */
const FOOTPRINT = TILE * 0.82;

/**
 * The Quaternius MegaKit ships three pre-built structures; we map them
 * to the three growth levels (small → mid → large) and tint them per
 * zone so R/C/I read differently while sharing the pack's look. Drop a
 * `models.json` in to override the level → filename mapping.
 */
const LEVEL_FILES: Record<number, string> = {
  1: "building_small.glb",
  2: "building_medium.glb",
  3: "building_large.glb",
};

/** Subtle per-zone multiply tint over the textured facades. */
const ZONE_TINT: Record<BuildingZone, number> = {
  res: 0xf3ddc8, // warm brownstone
  com: 0xc4d4ea, // cool steel/glass
  ind: 0xe7d49a, // industrial amber
};

/** Raw normalised model per level (white). */
const levelProtos = new Map<number, THREE.Object3D>();
/** Tinted, cached per (zone, level). */
const prototypes = new Map<string, THREE.Object3D>();

/**
 * Load the building GLBs under `basePath`. Missing files just leave the
 * procedural fallback in place. A `models.json` may remap levels:
 *   { "1": "MyHouse.glb", "2": "...", "3": "..." }
 */
export async function preloadKit(basePath = "/models/citykit/"): Promise<void> {
  const files = { ...LEVEL_FILES };
  try {
    const res = await fetch(basePath + "models.json");
    if (res.ok) {
      const map = (await res.json()) as Record<string, string>;
      for (const [lvl, file] of Object.entries(map)) {
        const n = Number(lvl);
        if (n >= 1 && n <= 3) files[n] = file;
      }
    }
  } catch {
    /* no manifest — use defaults */
  }
  const loader = new GLTFLoader();
  await Promise.all(
    Object.entries(files).map(
      ([lvl, file]) =>
        new Promise<void>((resolve) => {
          loader.load(
            basePath + file,
            (gltf) => {
              levelProtos.set(Number(lvl), normalizeToFootprint(gltf.scene));
              resolve();
            },
            undefined,
            () => resolve(), // missing/failed → procedural fallback
          );
        }),
    ),
  );
}

/** Build (once) the zone-tinted prototype for a level, cloning
 *  materials so the tint doesn't bleed across zones. */
function tintedProto(zone: BuildingZone, lvl: number): THREE.Object3D | null {
  const key = zone + lvl;
  const cached = prototypes.get(key);
  if (cached) return cached;
  const raw = levelProtos.get(lvl);
  if (!raw) return null;
  const tinted = raw.clone(true);
  const tint = new THREE.Color(ZONE_TINT[zone]);
  const tintMat = (m: THREE.Material): THREE.Material => {
    const c = m.clone() as THREE.MeshStandardMaterial;
    if (c.color) c.color.multiply(tint);
    return c;
  };
  tinted.traverse((o) => {
    const mesh = o as THREE.Mesh;
    if (!mesh.isMesh) return;
    mesh.material = Array.isArray(mesh.material)
      ? mesh.material.map(tintMat)
      : tintMat(mesh.material);
  });
  prototypes.set(key, tinted);
  return tinted;
}

/** Scale + recentre a loaded model so its base is at y=0 and it fits. */
function normalizeToFootprint(scene: THREE.Object3D): THREE.Object3D {
  scene.updateWorldMatrix(true, true);
  const box = new THREE.Box3().setFromObject(scene);
  const size = new THREE.Vector3();
  const center = new THREE.Vector3();
  box.getSize(size);
  box.getCenter(center);
  const span = Math.max(size.x, size.z) || 1;
  const scale = FOOTPRINT / span;
  const holder = new THREE.Group();
  scene.position.set(-center.x, -box.min.y, -center.z);
  scene.scale.setScalar(1);
  const inner = new THREE.Group();
  inner.add(scene);
  inner.scale.setScalar(scale);
  holder.add(inner);
  holder.traverse((o) => {
    const m = o as THREE.Mesh;
    if (m.isMesh) {
      m.castShadow = true;
      m.receiveShadow = true;
    }
  });
  return holder;
}

/**
 * Build a city block for the given zone + level at cell (gx,gz). The
 * cell coords seed deterministic variety so every client renders the
 * same skyline.
 */
export function makeBuilding(
  zone: BuildingZone,
  level: number,
  gx: number,
  gz: number,
): THREE.Object3D {
  const lvl = Math.max(1, Math.min(3, Math.round(level)));
  const proto = tintedProto(zone, lvl);
  const group = new THREE.Group();
  if (proto) {
    const inst = proto.clone(true);
    inst.rotation.y = (Math.floor(hash2(gx, gz, 7) * 4) * Math.PI) / 2;
    group.add(inst);
    return group;
  }
  proceduralMassing(group, zone, lvl, gx, gz);
  return group;
}

/**
 * Clean low-poly fallback in the spirit of the kit: flat-shaded solid
 * massing, a contrasting ground-floor podium and a darker roof slab —
 * no busy textures. Taller levels set back into a second tier.
 */
function proceduralMassing(
  group: THREE.Group,
  zone: BuildingZone,
  lvl: number,
  gx: number,
  gz: number,
): void {
  const style = ZONE[zone];
  const body = style.body[Math.floor(hash2(gx, gz, 3) * style.body.length)];
  const heights = [0, 3.2, 7.5, 15][lvl];
  const jitter = 0.85 + hash2(gx, gz, 11) * 0.3;
  const h = heights * jitter;
  const w = FOOTPRINT;

  const bodyMat = new THREE.MeshLambertMaterial({ color: body, flatShading: true });
  const roofMat = new THREE.MeshLambertMaterial({ color: darken(body, 0.62), flatShading: true });
  const podiumMat = new THREE.MeshLambertMaterial({ color: darken(body, 0.8), flatShading: true });

  // Ground-floor podium (storefront) so it doesn't read as a plain box.
  const podiumH = Math.min(1.4, h * 0.4);
  const podium = new THREE.Mesh(new THREE.BoxGeometry(w * 1.02, podiumH, w * 1.02), podiumMat);
  podium.position.y = podiumH / 2;
  podium.castShadow = podium.receiveShadow = true;
  group.add(podium);

  const tiers = lvl >= 3 ? 2 : 1;
  let baseY = podiumH;
  let tierW = w;
  for (let t = 0; t < tiers; t++) {
    const remaining = h - podiumH;
    const tierH = tiers === 1 ? remaining : t === 0 ? remaining * 0.62 : remaining * 0.38;
    const box = new THREE.Mesh(new THREE.BoxGeometry(tierW, tierH, tierW), bodyMat);
    box.position.y = baseY + tierH / 2;
    box.castShadow = box.receiveShadow = true;
    group.add(box);
    // Roof slab.
    const roof = new THREE.Mesh(new THREE.BoxGeometry(tierW * 1.04, 0.35, tierW * 1.04), roofMat);
    roof.position.y = baseY + tierH;
    roof.castShadow = true;
    group.add(roof);
    baseY += tierH;
    tierW *= 0.74;
  }
  group.rotation.y = (Math.floor(hash2(gx, gz, 7) * 4) * Math.PI) / 2;
}

function darken(hex: number, f: number): number {
  const r = Math.floor(((hex >> 16) & 0xff) * f);
  const g = Math.floor(((hex >> 8) & 0xff) * f);
  const b = Math.floor((hex & 0xff) * f);
  return (r << 16) | (g << 8) | b;
}
