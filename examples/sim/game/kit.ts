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

/** Footprint a building is normalised to fit within (leaves a margin). */
const FOOTPRINT = TILE * 0.82;

/** GLB filenames per (zone, level). Drop matching files in to upgrade. */
const KIT_FILES: Record<string, string> = {
  res1: "res_small.glb",
  res2: "res_mid.glb",
  res3: "res_tower.glb",
  com1: "com_small.glb",
  com2: "com_mid.glb",
  com3: "com_tower.glb",
  ind1: "ind_small.glb",
  ind2: "ind_mid.glb",
  ind3: "ind_tower.glb",
};

/** Friendly slot names accepted in models.json → internal keys. */
const SLOT_ALIAS: Record<string, string> = {
  res_small: "res1", res_mid: "res2", res_tower: "res3",
  com_small: "com1", com_mid: "com2", com_tower: "com3",
  ind_small: "ind1", ind_mid: "ind2", ind_tower: "ind3",
};

const prototypes = new Map<string, THREE.Object3D>();

/**
 * Load whatever GLBs exist under `basePath`. Missing files are skipped
 * silently — those (zone, level)s fall back to procedural massing.
 *
 * An optional `models.json` in the folder remaps slots to filenames, so
 * you can drop the megakit's original-named GLBs in without renaming:
 *   { "res_tower": "SM_Bld_Apartment_03.glb", ... }
 * keyed by the same slots as KIT_FILES (res1 → "res1" or "res_small").
 */
export async function preloadKit(basePath = "/models/citykit/"): Promise<void> {
  const files = { ...KIT_FILES };
  try {
    const res = await fetch(basePath + "models.json");
    if (res.ok) {
      const map = (await res.json()) as Record<string, string>;
      for (const [slot, file] of Object.entries(map)) {
        // Accept either slot key ("res1") or filename key ("res_small").
        const key = SLOT_ALIAS[slot] ?? slot;
        if (key in files) files[key] = file;
      }
    }
  } catch {
    /* no manifest — use defaults */
  }
  const loader = new GLTFLoader();
  await Promise.all(
    Object.entries(files).map(
      ([key, file]) =>
        new Promise<void>((resolve) => {
          loader.load(
            basePath + file,
            (gltf) => {
              const proto = normalizeToFootprint(gltf.scene);
              prototypes.set(key, proto);
              resolve();
            },
            undefined,
            () => resolve(), // missing/failed → procedural fallback
          );
        }),
    ),
  );
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
  const proto = prototypes.get(zone + lvl);
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
