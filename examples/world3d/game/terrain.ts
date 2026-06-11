/**
 * Procedural island heightfield.
 *
 * One 257×257 heightmap drives everything: the terrain mesh (single
 * draw call, vertex-colored), player collision (bilinear heightAt),
 * vegetation placement rules (height/slope masks), and the water
 * shader's shoreline foam (heights uploaded as a DataTexture).
 */
import * as THREE from "three";
import { SEED, TERRAIN_RES, WATER_LEVEL, WORLD_HALF, WORLD_SIZE } from "./config";
import { Simplex2 } from "./noise";

// Ground tones sit close to the grass-blade shader colors so the
// instanced grass field reads as part of the terrain, not a decal.
const COLOR_SAND = new THREE.Color("#c9b488");
const COLOR_GRASS_LOW = new THREE.Color("#5a7330");
const COLOR_GRASS_HIGH = new THREE.Color("#3f5c22");
const COLOR_DIRT = new THREE.Color("#6f5f43");
const COLOR_ROCK = new THREE.Color("#83817c");
const COLOR_SEABED = new THREE.Color("#94896a");

export class Terrain {
  readonly mesh: THREE.Mesh;
  /** Row-major heights, TERRAIN_RES × TERRAIN_RES, world y in meters. */
  readonly heights: Float32Array;

  private readonly step = WORLD_SIZE / (TERRAIN_RES - 1);
  private readonly noise = new Simplex2(SEED);
  private readonly detail = new Simplex2(SEED ^ 0x9e3779b9);

  constructor(detailMap?: THREE.Texture) {
    this.heights = this.generateHeights();
    this.mesh = this.buildMesh(detailMap);
  }

  private generateHeights(): Float32Array {
    const res = TERRAIN_RES;
    const heights = new Float32Array(res * res);
    for (let j = 0; j < res; j++) {
      for (let i = 0; i < res; i++) {
        const x = -WORLD_HALF + i * this.step;
        const z = -WORLD_HALF + j * this.step;
        heights[j * res + i] = this.computeHeight(x, z);
      }
    }
    return heights;
  }

  /** Analytic height function — only used during generation. */
  private computeHeight(x: number, z: number): number {
    const nx = x / WORLD_SIZE;
    const nz = z / WORLD_SIZE;

    // Radial island falloff with noisy coastline.
    const wobble = this.noise.fbm(nx * 3 + 7.3, nz * 3 - 2.1, 3) * 0.16;
    const r = Math.hypot(nx, nz) * 2 + wobble; // 0 center → ~1 edge
    const falloff = Math.max(0, 1 - Math.pow(Math.max(0, r), 2.2));

    // Rolling base hills + a ridged mountain spine through the middle.
    const hills = this.noise.fbm(nx * 4, nz * 4, 5) * 0.5 + 0.5; // [0,1]
    const spine = this.noise.ridged(nx * 2.2 + 3.7, nz * 2.2 - 1.4, 4); // ~[0,1]
    const mountainMask = Math.pow(falloff, 1.6);

    let h = -7 // seabed baseline so the coast actually submerges
      + falloff * (6 + hills * 14)
      + mountainMask * Math.pow(spine, 2.1) * 46;

    // Fine detail, fades out underwater so the seabed stays smooth.
    const detail = this.detail.fbm(nx * 24, nz * 24, 3) * 0.9;
    h += detail * Math.min(1, Math.max(0, h * 0.5 + 0.5));

    // Flatten beaches: compress heights near sea level for walkable shores.
    if (h > -1 && h < 3) h = -1 + (h + 1) * 0.55;

    return h;
  }

  /**
   * Terraform a building pad: flatten the heightfield to the inner
   * area's average height, blending back to natural terrain across a
   * smoothstep skirt. Call refreshGeometry() after the last pad.
   * Deterministic (pure function of the call args), so every client
   * carves identical pads.
   */
  flattenArea(cx: number, cz: number, radius: number, blendRadius: number): number {
    const res = TERRAIN_RES;
    // Pad elevation: average of the inner disc.
    let sum = 0;
    let n = 0;
    for (let dz = -radius; dz <= radius; dz += this.step) {
      for (let dx = -radius; dx <= radius; dx += this.step) {
        if (dx * dx + dz * dz > radius * radius) continue;
        sum += this.heightAt(cx + dx, cz + dz);
        n++;
      }
    }
    const target = n > 0 ? sum / n : this.heightAt(cx, cz);

    const reach = radius + blendRadius;
    const i0 = Math.max(0, Math.floor((cx - reach + WORLD_HALF) / this.step));
    const i1 = Math.min(res - 1, Math.ceil((cx + reach + WORLD_HALF) / this.step));
    const j0 = Math.max(0, Math.floor((cz - reach + WORLD_HALF) / this.step));
    const j1 = Math.min(res - 1, Math.ceil((cz + reach + WORLD_HALF) / this.step));
    for (let j = j0; j <= j1; j++) {
      for (let i = i0; i <= i1; i++) {
        const x = -WORLD_HALF + i * this.step;
        const z = -WORLD_HALF + j * this.step;
        const d = Math.hypot(x - cx, z - cz);
        if (d > reach) continue;
        // 1 inside the pad, smoothstep down to 0 at the skirt edge.
        const t = d <= radius ? 1 : 1 - (d - radius) / blendRadius;
        const s = t * t * (3 - 2 * t);
        const idx = j * res + i;
        this.heights[idx] = this.heights[idx] * (1 - s) + target * s;
      }
    }
    return target;
  }

  /** Re-apply heights/colors/normals to the existing mesh after edits. */
  refreshGeometry() {
    const res = TERRAIN_RES;
    const geo = this.mesh.geometry;
    const pos = geo.attributes.position as THREE.BufferAttribute;
    const colors = geo.attributes.color as THREE.BufferAttribute;
    const c = new THREE.Color();
    const tint = new Simplex2(SEED ^ 0x51ab);
    for (let v = 0; v < pos.count; v++) {
      const x = pos.getX(v);
      const z = pos.getZ(v);
      const i = Math.round((x + WORLD_HALF) / this.step);
      const j = Math.round((z + WORLD_HALF) / this.step);
      const h = this.heights[j * res + i];
      pos.setY(v, h);
      this.pickColor(c, h, this.slopeAt(x, z), tint.noise(x * 0.02, z * 0.02));
      colors.setXYZ(v, c.r, c.g, c.b);
    }
    pos.needsUpdate = true;
    colors.needsUpdate = true;
    geo.computeVertexNormals();
  }

  /** Bilinear height sample at any world (x, z). */
  heightAt(x: number, z: number): number {
    const res = TERRAIN_RES;
    const fx = (x + WORLD_HALF) / this.step;
    const fz = (z + WORLD_HALF) / this.step;
    const i = Math.max(0, Math.min(res - 2, Math.floor(fx)));
    const j = Math.max(0, Math.min(res - 2, Math.floor(fz)));
    const tx = Math.max(0, Math.min(1, fx - i));
    const tz = Math.max(0, Math.min(1, fz - j));
    const h00 = this.heights[j * res + i];
    const h10 = this.heights[j * res + i + 1];
    const h01 = this.heights[(j + 1) * res + i];
    const h11 = this.heights[(j + 1) * res + i + 1];
    return (
      h00 * (1 - tx) * (1 - tz) +
      h10 * tx * (1 - tz) +
      h01 * (1 - tx) * tz +
      h11 * tx * tz
    );
  }

  /** Approximate surface normal via central differences. */
  normalAt(x: number, z: number, out = new THREE.Vector3()): THREE.Vector3 {
    const e = this.step;
    const hl = this.heightAt(x - e, z);
    const hr = this.heightAt(x + e, z);
    const hd = this.heightAt(x, z - e);
    const hu = this.heightAt(x, z + e);
    return out.set(hl - hr, 2 * e, hd - hu).normalize();
  }

  /** Slope in [0,1]: 0 = flat, 1 = vertical. */
  slopeAt(x: number, z: number): number {
    return 1 - this.normalAt(x, z).y;
  }

  /**
   * Heights packed for the water shader's shoreline foam: a single-
   * channel float texture sampled in world space.
   */
  buildHeightTexture(): THREE.DataTexture {
    const tex = new THREE.DataTexture(
      this.heights,
      TERRAIN_RES,
      TERRAIN_RES,
      THREE.RedFormat,
      THREE.FloatType,
    );
    tex.magFilter = THREE.LinearFilter;
    tex.minFilter = THREE.LinearFilter;
    tex.needsUpdate = true;
    return tex;
  }

  private buildMesh(detailMap?: THREE.Texture): THREE.Mesh {
    const res = TERRAIN_RES;
    const geo = new THREE.PlaneGeometry(WORLD_SIZE, WORLD_SIZE, res - 1, res - 1);
    geo.rotateX(-Math.PI / 2); // y-up, vertices now ordered +z rows

    const pos = geo.attributes.position as THREE.BufferAttribute;
    const colors = new Float32Array(pos.count * 3);
    const c = new THREE.Color();
    const tint = new Simplex2(SEED ^ 0x51ab);

    for (let v = 0; v < pos.count; v++) {
      const x = pos.getX(v);
      const z = pos.getZ(v);
      // PlaneGeometry's grid aligns with ours; use the exact stored
      // height so collision and rendering agree everywhere.
      const i = Math.round((x + WORLD_HALF) / this.step);
      const j = Math.round((z + WORLD_HALF) / this.step);
      const h = this.heights[j * res + i];
      pos.setY(v, h);

      const slope = this.slopeAt(x, z);
      this.pickColor(c, h, slope, tint.noise(x * 0.02, z * 0.02));
      colors[v * 3] = c.r;
      colors[v * 3 + 1] = c.g;
      colors[v * 3 + 2] = c.b;
    }

    geo.setAttribute("color", new THREE.BufferAttribute(colors, 3));
    geo.computeVertexNormals();

    const mat = new THREE.MeshStandardMaterial({
      vertexColors: true,
      roughness: 0.96,
      metalness: 0,
      // High-frequency albedo speckle multiplied over the vertex
      // colors — the difference between "vector art" and ground.
      map: detailMap ?? null,
    });
    const mesh = new THREE.Mesh(geo, mat);
    mesh.receiveShadow = true;
    mesh.name = "terrain";
    return mesh;
  }

  private pickColor(out: THREE.Color, h: number, slope: number, tintNoise: number) {
    if (h < WATER_LEVEL - 0.5) {
      out.copy(COLOR_SEABED);
    } else if (h < WATER_LEVEL + 1.6) {
      out.copy(COLOR_SAND);
    } else if (h < WATER_LEVEL + 2.6) {
      // Sand → grass blend band.
      out.copy(COLOR_SAND).lerp(COLOR_GRASS_LOW, (h - WATER_LEVEL - 1.6) / 1.0);
    } else {
      const t = Math.min(1, (h - 2.6) / 30);
      out.copy(COLOR_GRASS_LOW).lerp(COLOR_GRASS_HIGH, t);
      if (h > 26) out.lerp(COLOR_ROCK, Math.min(1, (h - 26) / 12));
    }
    // Steep faces read as dirt/rock regardless of altitude.
    if (slope > 0.18) out.lerp(COLOR_DIRT, Math.min(1, (slope - 0.18) * 4));
    if (slope > 0.34) out.lerp(COLOR_ROCK, Math.min(1, (slope - 0.34) * 5));
    // Large-scale tint variation breaks up flat fields of color.
    out.offsetHSL(0, 0, tintNoise * 0.03);
  }

  dispose() {
    this.mesh.geometry.dispose();
    (this.mesh.material as THREE.Material).dispose();
  }
}
