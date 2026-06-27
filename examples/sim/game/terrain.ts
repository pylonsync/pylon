/**
 * Terrain — a deterministic heightfield with a buildable central basin,
 * mountains ringing the map, a winding river and a lake. Everything in
 * the world (buildings, roads, trees, the placement cursor) reads its Y
 * from heightAt(), so the city drapes over real land.
 *
 * The whole field is a pure function of world (x,z) + a fixed seed, so
 * every client renders the identical landscape.
 */
import * as THREE from "three";
import { GRID, TILE, TERRAIN_SEED, WATER_LEVEL, WORLD_HALF } from "./config";
import { hash2 } from "./prng";
import { makeGroundTexture, makeWaterNormal } from "./textures";

const smoothstep = (a: number, b: number, x: number): number => {
  const t = Math.max(0, Math.min(1, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
};
const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

/** Smooth value noise from the integer hash. */
function valueNoise(x: number, z: number, seed: number): number {
  const x0 = Math.floor(x);
  const z0 = Math.floor(z);
  const fx = x - x0;
  const fz = z - z0;
  const u = fx * fx * (3 - 2 * fx);
  const v = fz * fz * (3 - 2 * fz);
  const a = hash2(x0, z0, seed);
  const b = hash2(x0 + 1, z0, seed);
  const c = hash2(x0, z0 + 1, seed);
  const d = hash2(x0 + 1, z0 + 1, seed);
  return lerp(lerp(a, b, u), lerp(c, d, u), v);
}

function fbm(x: number, z: number, seed: number, oct: number): number {
  let sum = 0;
  let amp = 0.5;
  let freq = 1;
  let norm = 0;
  for (let i = 0; i < oct; i++) {
    sum += amp * valueNoise(x * freq, z * freq, seed + i * 17);
    norm += amp;
    amp *= 0.5;
    freq *= 2;
  }
  return sum / norm;
}

// River centreline (z as a function of x) — kept north of the centre so
// it frames the starter city rather than flooding it.
function riverZ(x: number): number {
  return 92 + Math.sin(x * 0.011) * 64 + Math.sin(x * 0.031 + 1.7) * 24;
}

const LAKE = { x: 150, z: -150, r: 78 };

/** Terrain height (m) at world (x,z). May be below WATER_LEVEL (water). */
export function heightAt(x: number, z: number): number {
  const nx = x * 0.0016;
  const nz = z * 0.0016;
  const d = Math.hypot(x, z) / WORLD_HALF;

  // Rolling base.
  let h = (fbm(nx * 6, nz * 6, TERRAIN_SEED, 4) - 0.45) * 26;

  // Ridged mountains, ramped in toward the edges.
  const ridged = 1 - Math.abs(fbm(nx * 3 + 11, nz * 3 + 7, TERRAIN_SEED + 91, 3) * 2 - 1);
  h += ridged * smoothstep(0.42, 1.05, d) * 135;

  // Flatten a WIDE, near-level central plateau so the city is buildable and
  // roads/buildings sit cohesively on level ground. The mountains and water
  // stay outside this core.
  const basin = 1 - smoothstep(0.08, 0.5, d);
  const base = 5 + (fbm(nx * 8, nz * 8, TERRAIN_SEED, 2) - 0.5) * 1.3;
  h = lerp(h, base, basin);

  // River channel.
  h += -smoothstep(30, 0, Math.abs(z - riverZ(x))) * 24;

  // Lake.
  h += -smoothstep(LAKE.r, 0, Math.hypot(x - LAKE.x, z - LAKE.z)) * 26;

  return h;
}

/** Approximate slope (rise over run) at (x,z). */
export function slopeAt(x: number, z: number): number {
  const e = 3;
  const dx = heightAt(x + e, z) - heightAt(x - e, z);
  const dz = heightAt(x, z + e) - heightAt(x, z - e);
  return Math.hypot(dx, dz) / (2 * e);
}

export function isWaterAt(x: number, z: number): boolean {
  return heightAt(x, z) < WATER_LEVEL;
}

/** A cell is buildable if it's dry land and not too steep. */
export function isBuildableCell(cx: number, cz: number): boolean {
  return heightAt(cx, cz) > WATER_LEVEL + 0.3 && slopeAt(cx, cz) < 0.9;
}

/** Displaced, vertex-coloured terrain mesh covering the whole map. */
export function buildTerrainMesh(): THREE.Mesh {
  const size = GRID * TILE;
  const segs = 256;
  const geo = new THREE.PlaneGeometry(size, size, segs, segs);
  geo.rotateX(-Math.PI / 2);
  const pos = geo.attributes.position as THREE.BufferAttribute;
  const colors = new Float32Array(pos.count * 3);

  // The tint multiplies the grass map. Two grass tones — a lusher,
  // slightly darker green and a drier, paler one — blended by broad
  // low-frequency noise give the large lush/dry patches a real CS map
  // has, instead of one flat fully-saturated green carpet. Both sit a
  // little under white so the canopy reads muted, not neon.
  const cGrassLush = new THREE.Color(0xbccb9c);
  const cGrassDry = new THREE.Color(0xdcd8b8);
  const cSand = new THREE.Color(0xcbb98c);
  const cRock = new THREE.Color(0x83796a);
  const cSnow = new THREE.Color(0xf2f4f7);
  const tmp = new THREE.Color();

  for (let i = 0; i < pos.count; i++) {
    const x = pos.getX(i);
    const z = pos.getZ(i);
    const h = heightAt(x, z);
    pos.setY(i, h);
    const slope = slopeAt(x, z);

    // Broad lush↔dry grass variation (large tonal patches across the map).
    const gn = Math.min(1, Math.max(0, fbm(x * 0.035 + 13, z * 0.035 + 7, TERRAIN_SEED + 200, 3)));
    tmp.copy(cGrassLush).lerp(cGrassDry, gn);
    if (h < WATER_LEVEL + 1.2) tmp.lerp(cSand, smoothstep(WATER_LEVEL + 1.2, WATER_LEVEL - 1, h));
    if (slope > 0.5) tmp.lerp(cRock, smoothstep(0.5, 1.1, slope));
    if (h > 75) tmp.lerp(cSnow, smoothstep(75, 110, h));
    colors[i * 3] = tmp.r;
    colors[i * 3 + 1] = tmp.g;
    colors[i * 3 + 2] = tmp.b;
  }
  geo.setAttribute("color", new THREE.BufferAttribute(colors, 3));
  geo.computeVertexNormals();

  const tex = makeGroundTexture();
  tex.repeat.set(size / 16, size / 16);
  const mat = new THREE.MeshStandardMaterial({
    map: tex,
    vertexColors: true,
    roughness: 1,
    metalness: 0,
  });
  const mesh = new THREE.Mesh(geo, mat);
  mesh.receiveShadow = true;
  mesh.name = "terrain";
  return mesh;
}

/** Reflective water plane at sea level, depth-tinted at the shore. */
export function buildWater(): THREE.Mesh {
  const size = GRID * TILE * 1.5;
  const geo = new THREE.PlaneGeometry(size, size, 160, 160);
  geo.rotateX(-Math.PI / 2);

  // Depth tint: pale teal where the terrain almost reaches the waterline
  // (shallows), deepening to blue where it drops away — the shelving
  // coastline CS water has, instead of one flat slab. The shallows also
  // let the sandy bottom show through the (translucent) surface.
  const pos = geo.attributes.position as THREE.BufferAttribute;
  const colors = new Float32Array(pos.count * 3);
  const shallow = new THREE.Color(0x73b6c6);
  const deep = new THREE.Color(0x21566f);
  const tmp = new THREE.Color();
  for (let i = 0; i < pos.count; i++) {
    const depth = WATER_LEVEL - heightAt(pos.getX(i), pos.getZ(i));
    tmp.copy(shallow).lerp(deep, smoothstep(0, 10, depth));
    colors[i * 3] = tmp.r;
    colors[i * 3 + 1] = tmp.g;
    colors[i * 3 + 2] = tmp.b;
  }
  geo.setAttribute("color", new THREE.BufferAttribute(colors, 3));

  // Scrolling wave normal map: fine, subtle ripples that catch the sun glint
  // (low roughness already reflects the sky env). Animated by City via offset.
  const normal = makeWaterNormal(256);
  normal.repeat.set(size / 16, size / 16);
  const mat = new THREE.MeshStandardMaterial({
    vertexColors: true,
    transparent: true,
    opacity: 0.85,
    roughness: 0.12,
    metalness: 0.15,
    normalMap: normal,
    normalScale: new THREE.Vector2(0.14, 0.14),
  });
  const mesh = new THREE.Mesh(geo, mat);
  mesh.position.y = WATER_LEVEL - 0.05;
  mesh.name = "water";
  return mesh;
}
