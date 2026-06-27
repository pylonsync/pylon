/**
 * Procedural foliage — palms, bushes, rocks, and a grass field.
 *
 * Everything is instanced: the whole vegetation layer costs five draw
 * calls regardless of instance counts (trunks, fronds, bushes, rocks,
 * grass). Placement is a deterministic jittered grid filtered by
 * height/slope/noise masks, so every client grows the same jungle.
 *
 * Wind: palms/bushes sway via a tiny onBeforeCompile vertex injection;
 * grass uses a dedicated shader with per-blade phase, gusts, and a
 * distance fade so the field dissolves instead of popping.
 */
import * as THREE from "three";
import { QUALITY, SEED, WATER_LEVEL, WORLD_HALF, WORLD_SIZE, type QualityPreset } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import { Simplex2 } from "./noise";
import { makeRng, type Rng } from "./prng";
import type { Terrain } from "./terrain";
import type { WorldTextures } from "./game";
import { buildSpecies } from "./eztrees";
import {
  buildBroadleafPlantGeometry,
  buildFernGeometry,
  buildRockGeometry,
  mergeGeometries,
} from "./trees";

export interface Placement {
  x: number;
  y: number;
  z: number;
  scale: number;
  rot: number;
}

/**
 * Jittered-grid scatter: one candidate per cell, accepted by the
 * filter. Deterministic, blue-noise-ish spacing without the cost of
 * real Poisson sampling.
 */
function scatter(
  rng: Rng,
  cell: number,
  filter: (x: number, z: number, r: Rng) => Placement | null,
): Placement[] {
  const out: Placement[] = [];
  for (let z = -WORLD_HALF + cell; z < WORLD_HALF - cell; z += cell) {
    for (let x = -WORLD_HALF + cell; x < WORLD_HALF - cell; x += cell) {
      const px = x + rng.range(-cell * 0.45, cell * 0.45);
      const pz = z + rng.range(-cell * 0.45, cell * 0.45);
      const p = filter(px, pz, rng);
      if (p) out.push(p);
    }
  }
  return out;
}

/** Inject sway into a standard material's vertex shader. */
function addWind(material: THREE.Material, strength: number, uniforms: { uWind: THREE.IUniform }) {
  material.onBeforeCompile = (shader) => {
    shader.uniforms.uWind = uniforms.uWind;
    shader.vertexShader = `uniform float uWind;\n` + shader.vertexShader.replace(
      "#include <begin_vertex>",
      `#include <begin_vertex>
      {
        // Sway grows with height above the instance origin; phase from
        // the instance's world position so trees don't move in lockstep.
        float h = max(0.0, position.y);
        #ifdef USE_INSTANCING
          vec2 seedPos = vec2(instanceMatrix[3][0], instanceMatrix[3][2]);
        #else
          vec2 seedPos = vec2(0.0);
        #endif
        float phase = seedPos.x * 0.35 + seedPos.y * 0.27;
        float sway = sin(uWind + phase) * ${strength.toFixed(3)} * h * h * 0.012;
        transformed.x += sway;
        transformed.z += sway * 0.6;
      }`,
    );
  };
}

// ---------------------------------------------------------------------------
// Geometry builders (one base mesh each, then instanced)
// ---------------------------------------------------------------------------

/** Palm spine: smooth quadratic lean, shared by trunk and crown. */
const PALM_HEIGHT = 8.6;
const PALM_LEAN = 1.5;
function palmSpine(t: number, out: THREE.Vector3): THREE.Vector3 {
  return out.set(PALM_LEAN * t * t, PALM_HEIGHT * t, PALM_LEAN * 0.35 * t * t);
}

/**
 * Curved palm trunk: ONE continuous tapered tube — rings sampled along
 * the spine and stitched into a single indexed geometry.
 */
function buildTrunkGeometry(): THREE.BufferGeometry {
  const rings = 10;
  const around = 7;
  const positions: number[] = [];
  const uvs: number[] = [];
  const indices: number[] = [];

  const center = new THREE.Vector3();
  for (let r = 0; r <= rings; r++) {
    const t = r / rings;
    palmSpine(t, center);
    const radius = 0.21 * (1 - t * 0.45);
    // Slight bulge at the base, like real palms.
    const bulge = t < 0.12 ? 1 + (0.12 - t) * 2.2 : 1;
    for (let a = 0; a <= around; a++) {
      const theta = (a / around) * Math.PI * 2;
      positions.push(
        center.x + Math.cos(theta) * radius * bulge,
        center.y,
        center.z + Math.sin(theta) * radius * bulge,
      );
      uvs.push(a / around, t * 6); // wrap bark texture ~6 times up the trunk
    }
  }
  for (let r = 0; r < rings; r++) {
    for (let a = 0; a < around; a++) {
      const i0 = r * (around + 1) + a;
      const i1 = i0 + around + 1;
      indices.push(i0, i1, i0 + 1, i0 + 1, i1, i1 + 1);
    }
  }

  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geo.setAttribute("uv", new THREE.Float32BufferAttribute(uvs, 2));
  geo.setIndex(indices);
  geo.computeVertexNormals();
  return geo;
}

/** Palm crown: drooping frond quads radiating from the EXACT trunk tip. */
function buildFrondGeometry(): THREE.BufferGeometry {
  const fronds = 12;
  const geos: THREE.BufferGeometry[] = [];
  const tip = palmSpine(1, new THREE.Vector3());
  const frondLen = 4.4;
  for (let f = 0; f < fronds; f++) {
    const plane = new THREE.PlaneGeometry(frondLen, 1.25, 7, 1);
    const pos = plane.attributes.position as THREE.BufferAttribute;
    // Arc up out of the crown then droop toward the tip; pinch the end.
    for (let v = 0; v < pos.count; v++) {
      const x = pos.getX(v); // -L/2 .. L/2 along frond
      const t = (x + frondLen / 2) / frondLen;
      const arc = Math.sin(t * Math.PI * 0.5) * 0.5 - Math.pow(t, 1.7) * 2.6;
      pos.setY(v, pos.getY(v) * (1 - t * 0.75) + arc);
      pos.setX(v, x + frondLen / 2); // root at origin
    }
    // Alternate fronds tilt slightly so the crown isn't one flat disc.
    plane.rotateZ(f % 2 === 0 ? 0.12 : -0.06);
    plane.rotateY((f / fronds) * Math.PI * 2 + (f % 3) * 0.13);
    plane.translate(tip.x, tip.y - 0.1, tip.z);
    geos.push(plane);
  }
  const merged = mergeGeometries(geos);
  for (const g of geos) g.dispose();
  return merged;
}

// ---------------------------------------------------------------------------
// Grass shader
// ---------------------------------------------------------------------------

const GRASS_VERT = /* glsl */ `
  uniform float uTime;
  uniform vec3 uPlayerPos;
  uniform float uFadeDist;
  attribute vec3 aOffset;
  attribute float aScale;
  attribute float aPhase;
  attribute float aTint;
  attribute float aLight;
  varying vec2 vUv;
  varying float vTint;
  varying float vHeight;
  varying float vCamDist;
  varying float vLight;

  void main() {
    vUv = uv;
    vHeight = uv.y;
    vTint = aTint;
    vLight = aLight;

    // Distance fade: shrink blades to nothing as they leave the radius,
    // so there's no popping and far vertices collapse (early-out rasterization).
    float dist = distance(aOffset.xz, uPlayerPos.xz);
    float fade = 1.0 - smoothstep(uFadeDist * 0.7, uFadeDist, dist);
    vec3 scaled = position * aScale * fade;

    // Wind: bend the tip, keep the root planted. Two frequencies +
    // a slow traveling gust front so the field moves in waves.
    float gust = sin(uTime * 0.5 + aOffset.x * 0.05 + aOffset.z * 0.03);
    float bend = sin(uTime * 1.8 + aPhase) + sin(uTime * 0.7 + aPhase * 1.7) * 0.5 + gust;
    scaled.x += bend * 0.16 * uv.y * uv.y;
    scaled.z += bend * 0.07 * uv.y * uv.y;

    vec3 world = scaled + aOffset;
    vCamDist = distance(world, cameraPosition);
    gl_Position = projectionMatrix * viewMatrix * vec4(world, 1.0);
  }
`;

const GRASS_FRAG = /* glsl */ `
  uniform vec3 uFogColor;
  uniform float uFogDensity;
  uniform vec3 uSunDir;
  uniform sampler2D uBlade;
  varying vec2 vUv;
  varying float vTint;
  varying float vHeight;
  varying float vCamDist;
  varying float vLight;

  void main() {
    // Clump sheet: a fan of baked curved blades with alpha gaps.
    vec4 blade = texture2D(uBlade, vUv);
    if (blade.a < 0.5) discard;
    // Texture carries the blade colors; per-instance tint varies the
    // clump, baked terrain N·L shades lee slopes (the single biggest
    // realism cue without true shadow sampling on grass).
    vec3 color = blade.rgb * (0.75 + vTint * 0.5);
    color *= 0.30 + 0.85 * vLight;

    // Sun-side warm tint on the tips.
    color += vec3(0.05, 0.04, 0.008) * vHeight * vLight;

    // Match the scene's exponential-squared fog so the field melts
    // into the same haze as the terrain.
    float fogFactor = 1.0 - exp(-uFogDensity * uFogDensity * vCamDist * vCamDist);
    color = mix(color, uFogColor, fogFactor);

    gl_FragColor = vec4(color, 1.0);
  }
`;

// ---------------------------------------------------------------------------
// Streamed grass field
// ---------------------------------------------------------------------------

const GRASS_CELL = 16; // meters per streaming cell

interface GrassCell {
  offsets: Float32Array; // xyz per blade
  scales: Float32Array;
  phases: Float32Array;
  tints: Float32Array;
  /** Baked terrain N·L vs the sun — hillside grass shades correctly. */
  lights: Float32Array;
  count: number;
}

// ---------------------------------------------------------------------------

export class Vegetation implements GameSystem {
  readonly name = "vegetation";
  readonly group = new THREE.Group();

  private readonly windUniform = { uWind: { value: 0 } };
  private readonly grassUniforms: Record<string, THREE.IUniform>;
  private readonly disposables: Array<{ dispose(): void }> = [];

  private clearingDiscs: Array<{ x: number; z: number; r: number }> = [];
  /** Every tree position (palms + all EZ species) — spawn-point checks. */
  readonly treeSpots: Array<{ x: number; z: number }> = [];

  private isCleared(x: number, z: number, pad = 0): boolean {
    return this.clearingDiscs.some((c) => Math.hypot(x - c.x, z - c.z) < c.r + pad);
  }

  /** True when no tree trunk stands within `r` meters of (x, z). */
  clearOfTrees(x: number, z: number, r: number): boolean {
    return !this.treeSpots.some((t) => Math.hypot(x - t.x, z - t.z) < r);
  }

  // Streamed grass state.
  private grassCells = new Map<number, GrassCell>();
  private grassGeo!: THREE.InstancedBufferGeometry;
  private grassCapacity = 0;
  private grassRadius = 0;
  private lastGrassCell = { x: Number.NaN, z: Number.NaN };

  constructor(
    terrain: Terrain,
    fog: THREE.FogExp2,
    sunDir: THREE.Vector3,
    textures: WorldTextures,
    /** Keep-out discs (building pads) — nothing grows inside. */
    clearings: Array<{ x: number; z: number; r: number }>,
    quality: QualityPreset = QUALITY.high,
  ) {
    const mask = new Simplex2(SEED ^ 0x7e9);
    this.clearingDiscs = clearings;
    const cleared = (x: number, z: number, pad = 0) => this.isCleared(x, z, pad);

    // The ash species is generated first so the palms can share its
    // photographic bark texture — one bark language across all wood.
    const ash = buildSpecies("Ash Medium", SEED ^ 0xb16, this.windUniform.uWind);
    const ezBark =
      (ash.trunkMat as THREE.MeshPhongMaterial).map ?? textures.bark;

    // --- Palms ---
    const palmSpots = scatter(makeRng(SEED ^ 0xa1), 11, (x, z, r) => {
      if (cleared(x, z, 2)) return null;
      const h = terrain.heightAt(x, z);
      const slope = terrain.slopeAt(x, z);
      if (h < WATER_LEVEL + 0.6 || h > WATER_LEVEL + 18 || slope > 0.24) return null;
      // Cluster palms into groves near the coast.
      const grove = mask.fbm(x * 0.012, z * 0.012, 2);
      if (grove < -0.08 || r.next() > 0.85) return null;
      return { x, y: h - 0.15, z, scale: r.range(0.65, 1.35), rot: r.range(0, Math.PI * 2) };
    });

    const trunkGeo = buildTrunkGeometry();
    const trunkMat = new THREE.MeshStandardMaterial({
      color: "#a3937e",
      roughness: 0.95,
      map: ezBark,
    });
    addWind(trunkMat, 0.4, this.windUniform);
    const trunks = this.makeInstances(trunkGeo, trunkMat, palmSpots, true);

    const frondGeo = buildFrondGeometry();
    const frondMat = new THREE.MeshStandardMaterial({
      color: "#ffffff", // per-instance color carries the actual green
      roughness: 0.9,
      side: THREE.DoubleSide,
      // Leaflet sheet + cutout → feathered silhouettes instead of
      // solid triangles. alphaTest keeps it on the opaque pass (no
      // sorting headaches, casts correct shadows).
      map: textures.frond,
      alphaTest: 0.4,
    });
    addWind(frondMat, 1.0, this.windUniform);
    const fronds = this.makeInstances(frondGeo, frondMat, palmSpots, true);
    // Olive-to-deep-green variation so groves don't read as one color.
    // The leaflet texture is mid-bright, so tints sit around 0.5
    // lightness to land at jungle green after the multiply.
    const frondTint = new THREE.Color();
    const tintRng = makeRng(SEED ^ 0xf60d);
    for (let i = 0; i < palmSpots.length; i++) {
      // Deep jungle greens matched to the EZ leaf tone range.
      frondTint.setHSL(
        0.27 + tintRng.range(-0.02, 0.02),
        tintRng.range(0.3, 0.42),
        tintRng.range(0.4, 0.52),
      );
      fronds.setColorAt(i, frondTint);
    }
    if (fronds.instanceColor) fronds.instanceColor.needsUpdate = true;

    // --- Jungle trees: EZ Tree species, two draw calls per species ---
    // Broadleaf interior canopy (ash), scattered through moist zones.
    const jungleSpots = scatter(makeRng(SEED ^ 0x771), 17, (x, z, r) => {
      if (cleared(x, z, 2)) return null;
      const h = terrain.heightAt(x, z);
      const slope = terrain.slopeAt(x, z);
      if (h < WATER_LEVEL + 2.5 || h > WATER_LEVEL + 26 || slope > 0.26) return null;
      const moisture = mask.fbm(x * 0.009 + 14.2, z * 0.009 - 6.8, 2);
      if (moisture < 0.0 || r.next() > 0.62) return null;
      return { x, y: h - 0.1, z, scale: r.range(0.55, 0.95), rot: r.range(0, Math.PI * 2) };
    });
    const jungleTrunks = this.makeInstances(ash.trunkGeo, ash.trunkMat, jungleSpots, true);
    const jungleCanopies = this.makeInstances(ash.leafGeo, ash.leafMat, jungleSpots, true);

    // Big spreading oaks as emergents above the ash layer.
    const emergentSpots = scatter(makeRng(SEED ^ 0x882), 46, (x, z, r) => {
      if (cleared(x, z, 2)) return null;
      const h = terrain.heightAt(x, z);
      const slope = terrain.slopeAt(x, z);
      if (h < WATER_LEVEL + 3 || h > WATER_LEVEL + 22 || slope > 0.2) return null;
      if (r.next() > 0.55) return null;
      return { x, y: h - 0.1, z, scale: r.range(0.6, 1.0), rot: r.range(0, Math.PI * 2) };
    });
    const oak = buildSpecies("Oak Medium", SEED ^ 0xe46, this.windUniform.uWind);
    const emergentTrunks = this.makeInstances(oak.trunkGeo, oak.trunkMat, emergentSpots, true);
    const emergentCanopies = this.makeInstances(oak.leafGeo, oak.leafMat, emergentSpots, true);

    // High slopes: stunted broadleafs (same ash species at scrub
    // scale) instead of conifers — alpine pines read out of place on
    // a tropical island, and reusing the species keeps the palette
    // coherent and saves two draw calls.
    const pineSpots = scatter(makeRng(SEED ^ 0x993), 14, (x, z, r) => {
      if (cleared(x, z, 2)) return null;
      const h = terrain.heightAt(x, z);
      const slope = terrain.slopeAt(x, z);
      if (h < WATER_LEVEL + 16 || h > WATER_LEVEL + 34 || slope > 0.34) return null;
      if (r.next() > 0.55) return null;
      return { x, y: h - 0.1, z, scale: r.range(0.3, 0.55), rot: r.range(0, Math.PI * 2) };
    });
    const pineTrunks = this.makeInstances(ash.trunkGeo, ash.trunkMat, pineSpots, true);
    const pineCanopies = this.makeInstances(ash.leafGeo, ash.leafMat, pineSpots, true);

    for (const s of [...palmSpots, ...jungleSpots, ...emergentSpots, ...pineSpots]) {
      this.treeSpots.push({ x: s.x, z: s.z });
    }

    // --- Ground ferns ---
    const fernSpots = scatter(makeRng(SEED ^ 0xfe2), 6, (x, z, r) => {
      if (cleared(x, z, 2)) return null;
      const h = terrain.heightAt(x, z);
      const slope = terrain.slopeAt(x, z);
      if (h < WATER_LEVEL + 1.2 || h > WATER_LEVEL + 24 || slope > 0.3) return null;
      if (mask.fbm(x * 0.014 + 4.4, z * 0.014 + 8.1, 2) < -0.1 || r.next() > 0.6) return null;
      return { x, y: h + 0.02, z, scale: r.range(0.6, 1.5), rot: r.range(0, Math.PI * 2) };
    });
    const fernGeo = buildFernGeometry();
    const fernMat = new THREE.MeshStandardMaterial({
      color: "#ffffff",
      roughness: 0.9,
      side: THREE.DoubleSide,
      map: textures.frond,
      alphaTest: 0.4,
    });
    addWind(fernMat, 0.9, this.windUniform);
    const ferns = this.makeInstances(fernGeo, fernMat, fernSpots, false, {
      hue: [0.27, 0.33],
      sat: [0.35, 0.5],
      light: [0.38, 0.58],
    });

    // --- Broadleaf ground plants ---
    const plantSpots = scatter(makeRng(SEED ^ 0x9a7), 8, (x, z, r) => {
      if (cleared(x, z, 2)) return null;
      const h = terrain.heightAt(x, z);
      const slope = terrain.slopeAt(x, z);
      if (h < WATER_LEVEL + 1.0 || h > WATER_LEVEL + 20 || slope > 0.28) return null;
      if (mask.fbm(x * 0.016 - 7.7, z * 0.016 + 2.9, 2) < -0.05 || r.next() > 0.55) return null;
      return { x, y: h + 0.02, z, scale: r.range(0.7, 1.7), rot: r.range(0, Math.PI * 2) };
    });
    const plantGeo = buildBroadleafPlantGeometry();
    const plantMat = new THREE.MeshStandardMaterial({
      color: "#ffffff",
      roughness: 0.85,
      side: THREE.DoubleSide,
      map: textures.leaf,
      alphaTest: 0.4,
    });
    addWind(plantMat, 0.7, this.windUniform);
    const plants = this.makeInstances(plantGeo, plantMat, plantSpots, false, {
      hue: [0.26, 0.32],
      sat: [0.4, 0.55],
      light: [0.45, 0.7],
    });

    // --- Bushes ---
    const bushSpots = scatter(makeRng(SEED ^ 0xb2), 10, (x, z, r) => {
      if (cleared(x, z, 2)) return null;
      const h = terrain.heightAt(x, z);
      const slope = terrain.slopeAt(x, z);
      if (h < WATER_LEVEL + 1.2 || h > WATER_LEVEL + 26 || slope > 0.3) return null;
      if (mask.fbm(x * 0.02 + 9.1, z * 0.02, 2) < -0.05 || r.next() > 0.45) return null;
      return { x, y: h + 0.2, z, scale: r.range(0.5, 1.6), rot: r.range(0, Math.PI * 2) };
    });
    // EZ Tree ships proper bush presets — same leaf-billboard art
    // style as the trees, so the understory matches the canopy.
    const bush = buildSpecies("Bush 2", SEED ^ 0xb05, this.windUniform.uWind);
    const bushTrunks = this.makeInstances(bush.trunkGeo, bush.trunkMat, bushSpots, true);
    const bushes = this.makeInstances(bush.leafGeo, bush.leafMat, bushSpots, true);

    // --- Rocks: angular, flat-shaded, half-buried, grey-brown variety ---
    const rockSpots = scatter(makeRng(SEED ^ 0xc4), 15, (x, z, r) => {
      if (cleared(x, z, 2)) return null;
      const h = terrain.heightAt(x, z);
      const slope = terrain.slopeAt(x, z);
      // Rocks favor slopes, ridgelines, and the shoreline.
      const shoreline = h > WATER_LEVEL - 1.5 && h < WATER_LEVEL + 2.5;
      if (h < WATER_LEVEL - 2) return null;
      if (!shoreline && slope < 0.14 && h < 18) return null;
      if (r.next() > (shoreline ? 0.35 : 0.5)) return null;
      const scale = r.range(0.5, shoreline ? 3.2 : 2.4);
      // Sink proportionally so rocks sit IN the ground, not on it.
      return { x, y: h - scale * 0.22, z, scale, rot: r.range(0, Math.PI * 2) };
    });
    const rockGeo = buildRockGeometry(SEED ^ 0xc5);
    const rockMat = new THREE.MeshStandardMaterial({
      color: "#ffffff",
      roughness: 0.97,
      map: textures.rock,
      // Relief lives in the normal map, not facets or polycount.
      normalMap: textures.rockNormal,
      normalScale: new THREE.Vector2(0.9, 0.9),
    });
    const rocks = this.makeInstances(rockGeo, rockMat, rockSpots, true, {
      hue: [0.55, 0.62], // cool slate, not olive
      sat: [0.02, 0.06],
      light: [0.32, 0.45],
    });

    // --- Grass field (streamed around the player) ---
    this.grassRadius = quality.grassRadius;
    this.grassUniforms = {
      uTime: { value: 0 },
      uPlayerPos: { value: new THREE.Vector3() },
      uFadeDist: { value: quality.grassRadius },
      uFogColor: { value: fog.color },
      uFogDensity: { value: fog.density },
      uSunDir: { value: sunDir },
      uBlade: { value: textures.grassClump },
    };
    const grass = this.buildGrass(terrain, mask, sunDir, quality);

    this.group.add(
      trunks,
      fronds,
      jungleTrunks,
      jungleCanopies,
      emergentTrunks,
      emergentCanopies,
      pineTrunks,
      pineCanopies,
      ferns,
      plants,
      bushTrunks,
      bushes,
      rocks,
      grass,
    );
    this.group.name = "vegetation";
  }

  private makeInstances(
    geo: THREE.BufferGeometry,
    mat: THREE.Material,
    spots: Placement[],
    castShadow: boolean,
    /** Optional per-instance HSL tint ranges (multiplied with the map). */
    tint?: { hue: [number, number]; sat: [number, number]; light: [number, number] },
  ): THREE.InstancedMesh {
    const mesh = new THREE.InstancedMesh(geo, mat, Math.max(1, spots.length));
    const m = new THREE.Matrix4();
    const q = new THREE.Quaternion();
    const up = new THREE.Vector3(0, 1, 0);
    const s = new THREE.Vector3();
    for (let i = 0; i < spots.length; i++) {
      const p = spots[i];
      q.setFromAxisAngle(up, p.rot);
      s.setScalar(p.scale);
      m.compose(new THREE.Vector3(p.x, p.y, p.z), q, s);
      mesh.setMatrixAt(i, m);
    }
    mesh.count = spots.length;
    mesh.instanceMatrix.needsUpdate = true;
    mesh.castShadow = castShadow;
    // Static world props: never re-upload matrices.
    mesh.instanceMatrix.setUsage(THREE.StaticDrawUsage);
    if (tint) {
      const c = new THREE.Color();
      const rng = makeRng(SEED ^ (spots.length * 2654435761));
      for (let i = 0; i < spots.length; i++) {
        c.setHSL(
          rng.range(tint.hue[0], tint.hue[1]),
          rng.range(tint.sat[0], tint.sat[1]),
          rng.range(tint.light[0], tint.light[1]),
        );
        mesh.setColorAt(i, c);
      }
      if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    }
    this.disposables.push(geo, mat, mesh);
    return mesh;
  }

  /**
   * Grass is streamed: blade placements for the whole island are
   * precomputed once into 16 m cells; every frame we check which cell
   * the player stands in, and on a cell change the blades within
   * grassRadius are memcpy'd into the live instance buffers. Near-field
   * density stays film-thick while the GPU only ever sees the local
   * patch — one draw call, fixed worst-case cost.
   */
  private buildGrass(
    terrain: Terrain,
    mask: Simplex2,
    sunDir: THREE.Vector3,
    quality: QualityPreset,
  ): THREE.Mesh {
    // Three crossed CLUMP cards per instance — each carries the baked
    // multi-blade clump texture, so one instance is a whole grass tuft.
    const cardA = new THREE.PlaneGeometry(0.95, 0.6, 1, 2);
    cardA.translate(0, 0.3, 0);
    const cardB = cardA.clone().rotateY(Math.PI / 3);
    const cardC = cardA.clone().rotateY((Math.PI * 2) / 3);
    const base = mergeGeometries([cardA, cardB, cardC]);
    cardA.dispose();
    cardB.dispose();
    cardC.dispose();

    // --- Precompute placements per cell, island-wide ---
    const rng = makeRng(SEED ^ 0xd6);
    const cellsPerSide = Math.ceil(WORLD_SIZE / GRASS_CELL);
    const perCellTarget = Math.round(GRASS_CELL * GRASS_CELL * quality.grassDensity);

    for (let cz = 0; cz < cellsPerSide; cz++) {
      for (let cx = 0; cx < cellsPerSide; cx++) {
        const baseX = -WORLD_HALF + cx * GRASS_CELL;
        const baseZ = -WORLD_HALF + cz * GRASS_CELL;
        // Cheap pre-filter: skip cells whose center can't grow grass.
        const centerH = terrain.heightAt(baseX + GRASS_CELL / 2, baseZ + GRASS_CELL / 2);
        if (centerH < WATER_LEVEL - 2 || centerH > WATER_LEVEL + 30) continue;

        // One clump-card instance per accepted point — the texture
        // carries the individual blades.
        const offsets = new Float32Array(perCellTarget * 3);
        const scales = new Float32Array(perCellTarget);
        const phases = new Float32Array(perCellTarget);
        const tints = new Float32Array(perCellTarget);
        const lights = new Float32Array(perCellTarget);
        const normal = new THREE.Vector3();
        let n = 0;
        for (let i = 0; i < perCellTarget; i++) {
          const x = baseX + rng.next() * GRASS_CELL;
          const z = baseZ + rng.next() * GRASS_CELL;
          if (this.isCleared(x, z, 1)) continue;
          const h = terrain.heightAt(x, z);
          const slope = terrain.slopeAt(x, z);
          if (h < WATER_LEVEL + 0.8 || h > WATER_LEVEL + 26 || slope > 0.3) continue;
          // Patchiness: meadows, not uniform turf.
          if (mask.fbm(x * 0.02 + 3.3, z * 0.02 - 1.8, 2) < -0.35) continue;
          offsets[n * 3] = x;
          offsets[n * 3 + 1] = h - 0.03;
          offsets[n * 3 + 2] = z;
          scales[n] = rng.range(0.55, 1.35);
          phases[n] = rng.range(0, Math.PI * 2);
          tints[n] = rng.next();
          // Bake terrain N·L so lee slopes read shaded (see GRASS_FRAG).
          lights[n] = Math.max(0, terrain.normalAt(x, z, normal).dot(sunDir));
          n++;
        }
        if (n > 0) {
          this.grassCells.set(cz * cellsPerSide + cx, {
            offsets,
            scales,
            phases,
            tints,
            lights,
            count: n,
          });
        }
      }
    }

    // --- Live instance buffers sized for the streamed radius ---
    const cellSpan = Math.ceil(this.grassRadius / GRASS_CELL);
    this.grassCapacity = (cellSpan * 2 + 1) ** 2 * perCellTarget;

    const geo = new THREE.InstancedBufferGeometry();
    geo.index = base.index;
    geo.setAttribute("position", base.attributes.position);
    geo.setAttribute("uv", base.attributes.uv);
    const mkAttr = (itemSize: number) =>
      new THREE.InstancedBufferAttribute(
        new Float32Array(this.grassCapacity * itemSize),
        itemSize,
      ).setUsage(THREE.DynamicDrawUsage);
    geo.setAttribute("aOffset", mkAttr(3));
    geo.setAttribute("aScale", mkAttr(1));
    geo.setAttribute("aPhase", mkAttr(1));
    geo.setAttribute("aTint", mkAttr(1));
    geo.setAttribute("aLight", mkAttr(1));
    geo.instanceCount = 0;
    this.grassGeo = geo;

    const mat = new THREE.ShaderMaterial({
      vertexShader: GRASS_VERT,
      fragmentShader: GRASS_FRAG,
      uniforms: this.grassUniforms,
      side: THREE.DoubleSide,
    });

    const mesh = new THREE.Mesh(geo, mat);
    mesh.frustumCulled = false; // streamed instances hug the player; fade culls in-shader
    mesh.name = "grass";
    this.disposables.push(geo, mat, base);
    return mesh;
  }

  /** Refill the live grass buffers from cells around the player. */
  private streamGrass(px: number, pz: number) {
    const cellsPerSide = Math.ceil(WORLD_SIZE / GRASS_CELL);
    const pcx = Math.floor((px + WORLD_HALF) / GRASS_CELL);
    const pcz = Math.floor((pz + WORLD_HALF) / GRASS_CELL);
    if (pcx === this.lastGrassCell.x && pcz === this.lastGrassCell.z) return;
    this.lastGrassCell = { x: pcx, z: pcz };

    const span = Math.ceil(this.grassRadius / GRASS_CELL);
    const offsets = this.grassGeo.getAttribute("aOffset") as THREE.InstancedBufferAttribute;
    const scales = this.grassGeo.getAttribute("aScale") as THREE.InstancedBufferAttribute;
    const phases = this.grassGeo.getAttribute("aPhase") as THREE.InstancedBufferAttribute;
    const tints = this.grassGeo.getAttribute("aTint") as THREE.InstancedBufferAttribute;
    const lights = this.grassGeo.getAttribute("aLight") as THREE.InstancedBufferAttribute;

    let n = 0;
    for (let cz = pcz - span; cz <= pcz + span; cz++) {
      if (cz < 0 || cz >= cellsPerSide) continue;
      for (let cx = pcx - span; cx <= pcx + span; cx++) {
        if (cx < 0 || cx >= cellsPerSide) continue;
        const cell = this.grassCells.get(cz * cellsPerSide + cx);
        if (!cell) continue;
        const take = Math.min(cell.count, this.grassCapacity - n);
        if (take <= 0) break;
        (offsets.array as Float32Array).set(cell.offsets.subarray(0, take * 3), n * 3);
        (scales.array as Float32Array).set(cell.scales.subarray(0, take), n);
        (phases.array as Float32Array).set(cell.phases.subarray(0, take), n);
        (tints.array as Float32Array).set(cell.tints.subarray(0, take), n);
        (lights.array as Float32Array).set(cell.lights.subarray(0, take), n);
        n += take;
      }
    }

    this.grassGeo.instanceCount = n;
    offsets.needsUpdate = true;
    scales.needsUpdate = true;
    phases.needsUpdate = true;
    tints.needsUpdate = true;
    lights.needsUpdate = true;
  }

  update(ctx: FrameCtx) {
    this.windUniform.uWind.value = ctx.time * 1.4;
    this.grassUniforms.uTime.value = ctx.time;
    (this.grassUniforms.uPlayerPos.value as THREE.Vector3).copy(ctx.playerPos);
    this.streamGrass(ctx.playerPos.x, ctx.playerPos.z);
  }

  dispose() {
    for (const d of this.disposables) d.dispose();
  }
}
