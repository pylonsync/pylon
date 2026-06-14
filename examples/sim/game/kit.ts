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
import { DRACOLoader } from "three/examples/jsm/loaders/DRACOLoader.js";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { TILE } from "./config";
import { ZONE } from "./config";
import { hash2 } from "./prng";

export type BuildingZone = "res" | "com" | "ind";

/** Footprint for the procedural fallback massing. */
const FOOTPRINT = TILE * 0.82;

/**
 * Each (zone, level) maps to its OWN building model with a footprint
 * target that grows with the level, so developing a tile swaps in a
 * visibly bigger, different building: houses / low blocks at level 1,
 * the tall MegaKit brownstones at level 3. `fp` is the fraction of a
 * cell the footprint fills. Drop a models.json in to override.
 */
const BUILDING_SLOTS: Record<string, { file: string; fp: number }> = {
  res1: { file: "House1.glb", fp: 0.6 },
  res2: { file: "Building1_Large.glb", fp: 0.92 },
  res3: { file: "building_medium.glb", fp: 0.95 },
  com1: { file: "Building2_Small.glb", fp: 0.64 },
  com2: { file: "Building2_Large.glb", fp: 0.9 },
  com3: { file: "building_large.glb", fp: 0.98 },
  ind1: { file: "Building3_Small.glb", fp: 0.7 },
  ind2: { file: "Building4.glb", fp: 0.88 },
  ind3: { file: "Building3_Big.glb", fp: 0.96 },
};

/**
 * The Quaternius low-poly building packs colour by MATERIAL NAME (no
 * textures), so we map those names to the palette here.
 */
const PALETTE: Array<[RegExp, number]> = [
  [/darkred/i, 0x6f2f2b],
  [/brickred/i, 0x9c4438],
  [/darkgrey|darkgray/i, 0x45474b],
  [/greyblue|grayblue/i, 0x66788a],
  [/whiteblue/i, 0xccd8e6],
  [/lightblue/i, 0x9cc0db],
  [/lightyellow/i, 0xe3d49a],
  [/beige/i, 0xd9c9a6],
  [/brown/i, 0x6b4a2e],
  [/white/i, 0xe8e5dc],
  [/grey|gray/i, 0x9a9aa0],
];
function paletteColor(name: string): number | null {
  for (const [re, hex] of PALETTE) if (re.test(name)) return hex;
  return null;
}

/** Normalised + coloured prototype per (zone, level) slot key. */
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
 * Load the building + road GLBs under `basePath`. Missing files just
 * leave the procedural fallback in place. A `models.json` may remap any
 * slot to a different filename: { "res1": "MyHouse.glb", ... }.
 */
export async function preloadKit(basePath = "/models/citykit/"): Promise<void> {
  const slots = structuredClone(BUILDING_SLOTS);
  try {
    const res = await fetch(basePath + "models.json");
    if (res.ok) {
      const map = (await res.json()) as Record<string, string>;
      for (const [slot, file] of Object.entries(map)) {
        if (slots[slot]) slots[slot].file = file;
      }
    }
  } catch {
    /* no manifest — use defaults */
  }
  // The building GLBs are Draco-compressed (~10x smaller); the decoder
  // streams from the standard three.js CDN. Non-Draco GLBs (roads) load
  // through the same loader unaffected.
  const draco = new DRACOLoader().setDecoderPath(
    "https://www.gstatic.com/draco/versioned/decoders/1.5.7/",
  );
  const loader = new GLTFLoader().setDRACOLoader(draco);
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
    ...Object.entries(slots).map(([key, { file, fp }]) =>
      loadInto(file, (g) => prototypes.set(key, normalizeToFootprint(g.scene, fp))),
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

/**
 * Recentre a model on the cell (base y=0, centred), scale so its
 * footprint fills `fpFraction` of a cell (the level → size progression),
 * and prepare materials: paint the name-keyed palette materials (the
 * houses/blocks ship grey) and matte the textured brownstones (which the
 * pack authors at metalness=1, so they'd otherwise render dark).
 */
function normalizeToFootprint(scene: THREE.Object3D, fpFraction: number): THREE.Object3D {
  scene.updateWorldMatrix(true, true);
  const box = new THREE.Box3().setFromObject(scene);
  const size = new THREE.Vector3();
  box.getSize(size);
  const center = new THREE.Vector3();
  box.getCenter(center);
  const span = Math.max(size.x, size.z) || 1;
  const scale = (TILE * fpFraction) / span;
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
    m.castShadow = true;
    m.receiveShadow = true;
    const mats = Array.isArray(m.material) ? m.material : [m.material];
    for (const mat of mats) {
      const std = mat as THREE.MeshStandardMaterial;
      if (!std.isMeshStandardMaterial) continue;
      if (std.map) {
        std.metalness = Math.min(std.metalness, 0.1);
        if (std.roughness < 0.5) std.roughness = 0.85;
      } else {
        const hex = paletteColor(std.name);
        if (hex !== null) std.color.setHex(hex);
        std.metalness = 0;
        std.roughness = 1;
        std.flatShading = true;
        std.needsUpdate = true;
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
  const proto = prototypes.get(zone + lvl);
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
