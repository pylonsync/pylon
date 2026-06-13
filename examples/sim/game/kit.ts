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

/** Road tile prototypes (filled to the cell), keyed straight/cross/tee/curve. */
const roadProtos = new Map<string, THREE.Object3D>();
const ROAD_FILES: Record<string, string> = {
  straight: "road_straight.glb",
  cross: "road_cross.glb",
  tee: "road_tee.glb",
  curve: "road_curve.glb",
};

/** Whether the GLB road tiles loaded (else tiles.ts uses procedural roads). */
export function hasRoadKit(): boolean {
  return roadProtos.has("straight");
}

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
  const loadInto = (
    file: string,
    onLoad: (gltf: { scene: THREE.Object3D }) => void,
  ): Promise<void> =>
    new Promise<void>((resolve) => {
      loader.load(
        basePath + file,
        (gltf) => {
          onLoad(gltf);
          resolve();
        },
        undefined,
        () => resolve(), // missing/failed → fallback
      );
    });

  await Promise.all([
    ...Object.entries(files).map(([lvl, file]) =>
      loadInto(file, (g) => levelProtos.set(Number(lvl), normalizeModel(g.scene))),
    ),
    // Road tiles fill exactly one cell (flat); see makeRoadTile.
    ...Object.entries(ROAD_FILES).map(([key, file]) =>
      loadInto(file, (g) => roadProtos.set(key, normalizeToCell(g.scene))),
    ),
  ]);
}

/** Recentre a flat tile so its footprint exactly fills one TILE cell,
 *  base at y=0. Used for the road pieces. */
function normalizeToCell(scene: THREE.Object3D): THREE.Object3D {
  scene.updateWorldMatrix(true, true);
  const box = new THREE.Box3().setFromObject(scene);
  const size = new THREE.Vector3();
  const center = new THREE.Vector3();
  box.getSize(size);
  box.getCenter(center);
  const span = Math.max(size.x, size.z) || 1;
  const scale = TILE / span;
  scene.position.set(-center.x, -box.min.y, -center.z);
  scene.scale.setScalar(1);
  const inner = new THREE.Group();
  inner.add(scene);
  inner.scale.setScalar(scale);
  const holder = new THREE.Group();
  holder.add(inner);
  holder.traverse((o) => {
    const m = o as THREE.Mesh;
    if (!m.isMesh) return;
    m.receiveShadow = true;
    // The pack authors asphalt as metalness=1, which renders dark with
    // odd specular tints under our env-map-less lighting. Asphalt is a
    // rough dielectric — force it matte so the grey surface + decals read.
    const mats = Array.isArray(m.material) ? m.material : [m.material];
    for (const mat of mats) {
      const std = mat as THREE.MeshStandardMaterial;
      if (std.isMeshStandardMaterial) {
        std.metalness = 0;
        std.roughness = 1;
      }
    }
  });
  return holder;
}

const laneMat = new THREE.MeshBasicMaterial({ color: 0xe8c34a });
const dashGeo = new THREE.PlaneGeometry(TILE * 0.04, TILE * 0.34).rotateX(-Math.PI / 2);

/**
 * One road tile for a cell — the MegaKit's clean asphalt road piece
 * (square, abuts its neighbours seamlessly) with a generated dashed
 * centre-line laid toward each connected neighbour, so straights, tees
 * and crossings all get continuous markings. (We use the plain asphalt
 * + our own markings because the kit's marked tile bakes red curb
 * decals, and its junction pieces are a 24 m module that doesn't align
 * with the 6 m straights.)
 */
export function makeRoadTile(
  n: boolean,
  e: boolean,
  s: boolean,
  w: boolean,
): THREE.Object3D | null {
  const proto = roadProtos.get("straight");
  if (!proto) return null;
  const group = new THREE.Group();
  group.add(proto.clone(true));

  // Centre-line dashes toward each connection (half a cell each).
  const y = 0.04;
  const off = TILE * 0.25;
  const addDash = (x: number, z: number, horizontal: boolean) => {
    const dash = new THREE.Mesh(dashGeo, laneMat);
    dash.position.set(x, y, z);
    if (horizontal) dash.rotation.y = Math.PI / 2;
    group.add(dash);
  };
  const conn = (n ? 1 : 0) + (e ? 1 : 0) + (s ? 1 : 0) + (w ? 1 : 0);
  if (conn === 0) {
    addDash(0, 0, false);
  } else {
    if (n) addDash(0, off, false);
    if (s) addDash(0, -off, false);
    if (e) addDash(off, 0, true);
    if (w) addDash(-off, 0, true);
  }
  return group;
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

/**
 * Recentre a loaded model on the cell (base at y=0, centred in x/z) and
 * apply the shared uniform KIT_SCALE so all models keep real-world
 * proportions relative to each other.
 */
function normalizeModel(scene: THREE.Object3D): THREE.Object3D {
  scene.updateWorldMatrix(true, true);
  const box = new THREE.Box3().setFromObject(scene);
  const center = new THREE.Vector3();
  box.getCenter(center);
  scene.position.set(-center.x, -box.min.y, -center.z);
  scene.scale.setScalar(1);
  const inner = new THREE.Group();
  inner.add(scene);
  inner.scale.setScalar(KIT_SCALE);
  const holder = new THREE.Group();
  holder.add(inner);
  holder.traverse((o) => {
    const m = o as THREE.Mesh;
    if (!m.isMesh) return;
    m.castShadow = true;
    m.receiveShadow = true;
    // The pack authors brick/concrete at metalness=1, which renders dark
    // and muddy without an env map. Knock metalness down so the facades
    // read as proper matte masonry (glass/metal trims stay a touch shiny).
    const mats = Array.isArray(m.material) ? m.material : [m.material];
    for (const mat of mats) {
      const std = mat as THREE.MeshStandardMaterial;
      if (std.isMeshStandardMaterial) {
        std.metalness = Math.min(std.metalness, 0.1);
        if (std.roughness < 0.5) std.roughness = 0.85;
      }
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
  faceRad?: number,
): THREE.Object3D {
  const lvl = Math.max(1, Math.min(3, Math.round(level)));
  // Models front onto -Z; faceRad orients +Z toward the road, so add PI
  // to turn the facade to the street. No road neighbour → deterministic
  // random rotation.
  const rotY =
    faceRad !== undefined
      ? faceRad + Math.PI
      : (Math.floor(hash2(gx, gz, 7) * 4) * Math.PI) / 2;
  const proto = tintedProto(zone, lvl);
  const group = new THREE.Group();
  if (proto) {
    const inst = proto.clone(true);
    inst.rotation.y = rotY;
    group.add(inst);
    return group;
  }
  proceduralMassing(group, zone, lvl, gx, gz, rotY);
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
  rotY: number,
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
  group.rotation.y = rotY;
}

function darken(hex: number, f: number): number {
  const r = Math.floor(((hex >> 16) & 0xff) * f);
  const g = Math.floor(((hex >> 8) & 0xff) * f);
  const b = Math.floor((hex & 0xff) * f);
  return (r << 16) | (g << 8) | b;
}
