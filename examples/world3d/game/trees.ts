/**
 * Additional procedural flora: broadleaf jungle trees, tall emergent
 * trees, ground ferns, and broadleaf plants. Each species is built as
 * one or two base geometries that Vegetation instances by the hundred —
 * the whole layer stays at a handful of draw calls.
 */
import * as THREE from "three";
import { Simplex2 } from "./noise";

/** Minimal BufferGeometry merge (positions/normals/uvs). */
export function mergeGeometries(geos: THREE.BufferGeometry[]): THREE.BufferGeometry {
  let vertCount = 0;
  let indexCount = 0;
  for (const g of geos) {
    vertCount += g.attributes.position.count;
    indexCount += g.index ? g.index.count : g.attributes.position.count;
  }
  const positions = new Float32Array(vertCount * 3);
  const normals = new Float32Array(vertCount * 3);
  const uvs = new Float32Array(vertCount * 2);
  const indices = new Uint32Array(indexCount);

  let vOff = 0;
  let iOff = 0;
  for (const g of geos) {
    const p = g.attributes.position as THREE.BufferAttribute;
    const n = g.attributes.normal as THREE.BufferAttribute | undefined;
    const uv = g.attributes.uv as THREE.BufferAttribute | undefined;
    positions.set(p.array as Float32Array, vOff * 3);
    if (n) normals.set(n.array as Float32Array, vOff * 3);
    if (uv) uvs.set(uv.array as Float32Array, vOff * 2);
    const count = g.index ? g.index.count : p.count;
    for (let i = 0; i < count; i++) {
      indices[iOff + i] = (g.index ? g.index.getX(i) : i) + vOff;
    }
    vOff += p.count;
    iOff += count;
  }

  const out = new THREE.BufferGeometry();
  out.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  out.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
  out.setAttribute("uv", new THREE.BufferAttribute(uvs, 2));
  out.setIndex(new THREE.BufferAttribute(indices, 1));
  return out;
}

/**
 * Ground fern: a rosette of small arcing fronds. Uses the palm-frond
 * leaflet texture, so it gets feathered edges for free.
 */
export function buildFernGeometry(): THREE.BufferGeometry {
  const fronds = 7;
  const geos: THREE.BufferGeometry[] = [];
  const len = 1.15;
  for (let f = 0; f < fronds; f++) {
    const plane = new THREE.PlaneGeometry(len, 0.34, 5, 1);
    const pos = plane.attributes.position as THREE.BufferAttribute;
    for (let v = 0; v < pos.count; v++) {
      const x = pos.getX(v);
      const t = (x + len / 2) / len;
      // Rise out of the ground then arc back down.
      pos.setY(v, pos.getY(v) * (1 - t * 0.6) + Math.sin(t * Math.PI) * 0.42);
      pos.setX(v, x + len / 2);
    }
    plane.rotateY((f / fronds) * Math.PI * 2 + (f % 2) * 0.3);
    geos.push(plane);
  }
  const merged = mergeGeometries(geos);
  for (const g of geos) g.dispose();
  return merged;
}

/**
 * Broadleaf ground plant: a whorl of wide drooping leaves (think
 * young banana / taro). Uses the oval leaf texture with alpha cutout.
 */
export function buildBroadleafPlantGeometry(): THREE.BufferGeometry {
  const leaves = 6;
  const geos: THREE.BufferGeometry[] = [];
  const len = 0.95;
  for (let f = 0; f < leaves; f++) {
    const plane = new THREE.PlaneGeometry(len, 0.42, 5, 1);
    const pos = plane.attributes.position as THREE.BufferAttribute;
    for (let v = 0; v < pos.count; v++) {
      const x = pos.getX(v);
      const t = (x + len / 2) / len;
      // Leaves start angled up, curl over at the tip.
      pos.setY(v, Math.sin(t * Math.PI * 0.75) * 0.55 - t * t * 0.18);
      pos.setX(v, x + len / 2);
    }
    plane.rotateZ(0.18);
    plane.rotateY((f / leaves) * Math.PI * 2 + (f % 3) * 0.21);
    geos.push(plane);
  }
  const merged = mergeGeometries(geos);
  for (const g of geos) g.dispose();
  return merged;
}

/**
 * Rock: indexed sphere, noise-displaced by vertex direction with a
 * couple of harder creases, flattened bottom so it sits buried in the
 * ground. Smooth normals + the rock normal map carry the surface
 * detail — real rocks aren't low-poly facets, they're smooth masses
 * with high-frequency relief.
 */
export function buildRockGeometry(seed: number): THREE.BufferGeometry {
  const geo = new THREE.SphereGeometry(1, 18, 12);
  const noise = new Simplex2(seed);
  const pos = geo.attributes.position as THREE.BufferAttribute;
  const dir = new THREE.Vector3();
  for (let v = 0; v < pos.count; v++) {
    dir.set(pos.getX(v), pos.getY(v), pos.getZ(v)).normalize();
    const broad = noise.fbm(dir.x * 1.6 + dir.y * 1.0 + 3.0, dir.z * 1.6 - dir.y * 0.7, 3);
    // Crease term: sharp ridges where the noise crosses zero.
    const crease = 1 - Math.abs(noise.noise(dir.x * 2.4 + 11.0, dir.z * 2.4 - 4.0));
    const r = 1 + broad * 0.34 + crease * crease * 0.12;
    pos.setXYZ(v, dir.x * r, Math.max(dir.y * r * 0.72, -0.2), dir.z * r);
  }
  geo.computeVertexNormals();
  return geo;
}
