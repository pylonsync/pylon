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
const BUILDING_SLOTS: Record<string, { files: string[]; fp: number }> = {
  res1: { files: ["House1.glb", "House2.glb"], fp: 0.6 },
  res2: { files: ["Building1_Large.glb", "Building2_Small.glb", "House2.glb", "building_small.glb"], fp: 0.9 },
  res3: { files: ["building_medium.glb", "building_large.glb", "Building1_Large.glb"], fp: 0.95 },
  com1: { files: ["Building2_Small.glb", "House2.glb", "building_small.glb"], fp: 0.66 },
  com2: { files: ["Building2_Large.glb", "Building4.glb", "building_medium.glb"], fp: 0.9 },
  com3: { files: ["building_large.glb", "building_medium.glb", "Building2_Large.glb"], fp: 0.98 },
  ind1: { files: ["Building3_Small.glb", "Building4.glb", "building_small.glb"], fp: 0.72 },
  ind2: { files: ["Building4.glb", "Building3_Big.glb", "Building2_Large.glb"], fp: 0.88 },
  ind3: { files: ["Building3_Big.glb", "building_large.glb", "building_medium.glb"], fp: 0.96 },
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

/** Normalised + coloured prototype variants per (zone, level) slot key. */
const prototypes = new Map<string, THREE.Object3D[]>();

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
      const map = (await res.json()) as Record<string, string | string[]>;
      for (const [slot, files] of Object.entries(map)) {
        if (slots[slot]) slots[slot].files = Array.isArray(files) ? files : [files];
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
    // Each slot loads all its variants in order; prototypes[slot] is the
    // variant array (makeBuilding picks one deterministically per cell).
    ...Object.entries(slots).flatMap(([key, { files, fp }]) => {
      prototypes.set(key, []);
      return files.map((file, i) =>
        loadInto(file, (g) => {
          const arr = prototypes.get(key)!;
          arr[i] = normalizeToFootprint(g.scene, fp);
        }),
      );
    }),
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
  // Slight overlap (×1.03) so neighbouring road tiles meet seamlessly
  // even where the terrain steps a little between cells.
  const scale = (TILE * 1.03) / span;
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

// Road paint is a lit (matte) material, not MeshBasic: basic is unlit, so
// markings would ignore the day/night cycle and glow against the dark night.
const laneMat = new THREE.MeshStandardMaterial({ color: 0xc7a83e, roughness: 1, metalness: 0 });
const dashGeo = new THREE.PlaneGeometry(TILE * 0.04, TILE * 0.34).rotateX(-Math.PI / 2);

// Sidewalk / kerb: a raised light-concrete strip laid along any tile edge
// that faces a non-road cell, so every block reads as a kerbed city block —
// the single strongest "this is a city, not models on grass" cue vs bare
// asphalt. Two box geometries (one per axis) shared across all road tiles.
const SW_W = TILE * 0.17; // strip width (~1.1 m)
const SW_H = 0.14; // raised kerb height
const sidewalkMat = new THREE.MeshStandardMaterial({
  color: 0xbcbcb4,
  roughness: 0.97,
  metalness: 0,
});
const swGeoNS = new THREE.BoxGeometry(TILE, SW_H, SW_W); // runs along x (N/S edges)
const swGeoEW = new THREE.BoxGeometry(SW_W, SW_H, TILE); // runs along z (E/W edges)

// Zebra crossing stripes — white bars laid on the approaches of a junction.
// Lit (matte) so they darken at night with the rest of the scene instead of
// glowing like a light source. Two bar geometries (one per road axis).
const crosswalkMat = new THREE.MeshStandardMaterial({ color: 0xcdcabf, roughness: 1, metalness: 0 });
const cwBarX = new THREE.PlaneGeometry(TILE * 0.46, TILE * 0.05).rotateX(-Math.PI / 2); // long in x
const cwBarZ = new THREE.PlaneGeometry(TILE * 0.05, TILE * 0.46).rotateX(-Math.PI / 2); // long in z

// Avenue median — a raised planted strip down the centre of an arterial's
// through-cells (a low grass kerb + a hedge), so major roads read as divided
// boulevards. Two geometries (one per road axis), shared across tiles.
const MEDIAN_W = TILE * 0.16;
const MEDIAN_H = 0.18;
const medianMat = new THREE.MeshStandardMaterial({ color: 0x5f8a44, roughness: 1 });
const medianGeoNS = new THREE.BoxGeometry(MEDIAN_W, MEDIAN_H, TILE); // along z (N/S avenue)
const medianGeoEW = new THREE.BoxGeometry(TILE, MEDIAN_H, MEDIAN_W); // along x (E/W avenue)
const hedgeMat = new THREE.MeshStandardMaterial({ color: 0x46732f, roughness: 1, flatShading: true });
const hedgeGeoNS = new THREE.BoxGeometry(MEDIAN_W * 0.7, 0.34, TILE * 0.82).translate(0, MEDIAN_H + 0.17, 0);
const hedgeGeoEW = new THREE.BoxGeometry(TILE * 0.82, 0.34, MEDIAN_W * 0.7).translate(0, MEDIAN_H + 0.17, 0);

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

  // Zebra crossings at junctions (tee or cross): a few bars on each
  // connected approach, near that edge, perpendicular to its road.
  if (conn >= 3) {
    const cwY = 0.045;
    const addBar = (geo: THREE.PlaneGeometry, x: number, z: number) => {
      const bar = new THREE.Mesh(geo, crosswalkMat);
      bar.position.set(x, cwY, z);
      group.add(bar);
    };
    for (let i = 0; i < 3; i++) {
      const d = TILE * (0.27 + i * 0.07);
      if (n) addBar(cwBarX, 0, d);
      if (s) addBar(cwBarX, 0, -d);
      if (e) addBar(cwBarZ, d, 0);
      if (w) addBar(cwBarZ, -d, 0);
    }
  }

  // Kerb every edge that faces a non-road cell (north = +z, east = +x).
  // The strips span the full cell so adjacent tiles' kerbs join into one
  // continuous sidewalk, and the gap left on a connected side is exactly
  // the mouth of that road. Corners overlap, which reads fine from above.
  const edge = TILE / 2 - SW_W / 2;
  const addWalk = (geo: THREE.BoxGeometry, x: number, z: number) => {
    const m = new THREE.Mesh(geo, sidewalkMat);
    m.position.set(x, SW_H / 2, z);
    m.castShadow = true;
    m.receiveShadow = true;
    group.add(m);
  };
  if (!n) addWalk(swGeoNS, 0, edge);
  if (!s) addWalk(swGeoNS, 0, -edge);
  if (!e) addWalk(swGeoEW, edge, 0);
  if (!w) addWalk(swGeoEW, -edge, 0);

  return group;
}

/**
 * A major arterial tile: a normal road tile (markings, kerbs, crosswalks)
 * plus a planted median down the centre of a straight through-cell, so
 * avenues read as divided boulevards distinct from the side streets.
 * Junctions get no median (the intersection stays open).
 */
export function makeAvenueTile(
  n: boolean,
  e: boolean,
  s: boolean,
  w: boolean,
): THREE.Object3D | null {
  const tile = makeRoadTile(n, e, s, w);
  if (!tile) return null;
  const throughNS = n && s && !e && !w;
  const throughEW = e && w && !n && !s;
  if (throughNS || throughEW) {
    const median = new THREE.Mesh(throughNS ? medianGeoNS : medianGeoEW, medianMat);
    median.position.set(0, MEDIAN_H / 2, 0);
    median.castShadow = true;
    median.receiveShadow = true;
    tile.add(median);
    const hedge = new THREE.Mesh(throughNS ? hedgeGeoNS : hedgeGeoEW, hedgeMat);
    hedge.castShadow = true;
    tile.add(hedge);
  }
  return tile;
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
  const variants = (prototypes.get(zone + lvl) ?? []).filter(Boolean);
  const proto = variants.length
    ? variants[Math.floor(hash2(gx, gz, 5) * variants.length) % variants.length]
    : undefined;
  // Seat the building against its street frontage (CS buildings line the
  // pavement) instead of centred in the lot: slide it toward the facing
  // road by its setback, leaving a small margin so it never overhangs the
  // kerb. Houses (small footprint) gain a clear frontage + backyard; lot-
  // filling towers barely move. Interior cells (no faceRad) stay centred.
  const fp = BUILDING_SLOTS[zone + lvl]?.fp ?? 0.8;
  const push =
    faceRad !== undefined ? Math.max(0, (TILE * (1 - fp)) / 2 - TILE * 0.06) : 0;
  const inner = new THREE.Group();
  inner.position.set(Math.sin(faceRad ?? 0) * push, 0, Math.cos(faceRad ?? 0) * push);
  // Vary height per building so the downtown steps into a varied skyline
  // instead of a uniform wall of equal boxes — strongest on the taller
  // levels, barely perceptible on houses. (Composes with the rise animation,
  // which scales the OUTER group's Y to 1.) Front of the inner group is the
  // building base, so this grows it upward.
  inner.scale.y = lvl >= 2 ? 0.82 + hash2(gx, gz, 17) * 0.46 : 0.94 + hash2(gx, gz, 17) * 0.14;
  if (proto) {
    const inst = proto.clone(true);
    inst.rotation.y = rotY;
    inner.add(inst);
  } else {
    proceduralMassing(inner, zone, lvl, gx, gz, rotY);
  }
  varyBuildingColor(inner, gx, gz, zone);
  const group = new THREE.Group();
  group.add(inner);
  // Rooftop clutter (AC/vent boxes) on the tall downtown towers only — a
  // close-up detail. L3 is the fewest, so the per-building Box3 to find the
  // roof top is paid rarely. Added to the OUTER group so the rise scales them
  // with the building.
  if (lvl >= 3) {
    const bb = new THREE.Box3().setFromObject(group);
    const topY = bb.max.y;
    const cx = (bb.min.x + bb.max.x) / 2;
    const cz = (bb.min.z + bb.max.z) / 2;
    const half = Math.min(bb.max.x - bb.min.x, bb.max.z - bb.min.z) * 0.5;
    if (Number.isFinite(topY) && half > 0.2) {
      const count = 1 + Math.floor(hash2(gx, gz, 30) * 2);
      for (let i = 0; i < count; i++) {
        const wd = half * (0.22 + hash2(gx, gz, 31 + i) * 0.18);
        const ac = new THREE.Mesh(new THREE.BoxGeometry(wd, wd * 0.5, wd), ROOFTOP_MAT);
        ac.position.set(
          cx + (hash2(gx, gz, 33 + i) - 0.5) * half,
          topY + wd * 0.25,
          cz + (hash2(gx, gz, 35 + i) - 0.5) * half,
        );
        ac.castShadow = true;
        group.add(ac);
      }
    }
  }
  return group;
}

const ROOFTOP_MAT = new THREE.MeshStandardMaterial({
  color: 0x9a9a98,
  roughness: 0.8,
  metalness: 0.3,
});

// Per-zone cast, blended over the per-building variation so districts read
// distinctly the way they do in Cities: Skylines: commercial trends lighter
// and cooler (concrete + glass), industrial darker and grimier, residential
// stays warm and varied (no cast).
const ZONE_MIX: Record<string, { target: THREE.Color; amt: number } | null> = {
  res: null,
  com: { target: new THREE.Color(0xdce4ec), amt: 0.32 },
  ind: { target: new THREE.Color(0xb8b2a6), amt: 0.26 },
};

// Per-building colour variation. Clones the (shared, prototype) materials
// and nudges brightness — plus a touch of hue on solid colours — keyed to
// the cell, so the same model repeated down a block reads as distinct
// buildings instead of a copy-paste wall, the way a CS street does.
// Textured facades keep their hue (value only) so brick stays brick.
const _bhsl = { h: 0, s: 0, l: 0 };
function varyBuildingColor(
  root: THREE.Object3D,
  gx: number,
  gz: number,
  zone: string,
): void {
  const hueShift = (hash2(gx, gz, 21) - 0.5) * 0.05;
  const valShift = 0.95 + hash2(gx, gz, 23) * 0.22; // lean brighter (0.95–1.17)
  const zoneMix = ZONE_MIX[zone] ?? null;
  const recolor = (mat: THREE.Material): THREE.Material => {
    const std = (mat as THREE.MeshStandardMaterial).clone();
    const name = (std.name || "").toLowerCase();
    if (name.includes("glass")) {
      // Office glazing ships as a near-black panel (#434343), which is what
      // makes the towers read as dark blocks. Make it a light reflective
      // blue curtain wall — the glass towers a CS downtown is built from.
      // A faint constant emissive reads as a sky reflection by day and as a
      // lit curtain wall at night (no per-frame work; day sun washes it out).
      std.color.setHex(0xa9c4d8);
      std.metalness = 0.45;
      std.roughness = 0.18;
      std.emissive = new THREE.Color(0x86a6c6);
      std.emissiveIntensity = 0.14;
    } else if (name.includes("interior")) {
      // The office "fake interior" panels behind the glass — make them emit
      // their own texture so rooms glow warm at night, lighting the downtown
      // skyline the way a CS night does. Subtle enough to vanish by day.
      std.emissive = new THREE.Color(0xffce92);
      std.emissiveMap = std.map ?? null;
      std.emissiveIntensity = 0.34;
    } else if (std.map) {
      // Textured brick / asphalt roof — its tint multiplies a dark map, so
      // BRIGHTEN it (>1) or tall facades + flat roofs read as black from
      // above. The office flat roofs (MI_Asphalt) + dark parapet trim are
      // the darkest thing in the aerial downtown, so lift them to a pale
      // concrete grey; brick facades only need a gentle lift.
      const roof =
        name.includes("asphalt") || name.includes("roof") || name.includes("trim_dark");
      std.color.setScalar((roof ? 2.2 : 1.1) + (hash2(gx, gz, 23) - 0.5) * 0.18);
    } else {
      std.color.getHSL(_bhsl);
      // Floor the lightness so dark roof/trim colours don't read as black
      // boxes from above. The brownstone slate roof (solid "DarkGrey") is the
      // darkest thing left in the core, so lift it harder to a mid slate grey.
      const slate = name.includes("darkgrey") || name.includes("darkgray");
      const l = Math.min(1, Math.max(slate ? 0.54 : 0.3, _bhsl.l * valShift));
      std.color.setHSL((_bhsl.h + hueShift + 1) % 1, _bhsl.s, l);
      if (zoneMix) std.color.lerp(zoneMix.target, zoneMix.amt);
    }
    return std;
  };
  root.traverse((o) => {
    const m = o as THREE.Mesh;
    if (!m.isMesh) return;
    // Preserve the single-vs-array shape: assigning a one-element array to a
    // mesh whose geometry has no material groups renders NOTHING.
    m.material = Array.isArray(m.material) ? m.material.map(recolor) : recolor(m.material);
  });
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
