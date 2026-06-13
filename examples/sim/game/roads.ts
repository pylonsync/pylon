/**
 * Procedural auto-tiling roads.
 *
 * Roads are generated, not imported: every road cell becomes an
 * asphalt quad plus lane markings chosen from its 4-neighbour
 * connection mask, so straights, corners, tees and crossroads emerge
 * automatically and tile seamlessly on the grid. The whole network is
 * one merged mesh (rebuilt when the road set changes) — a couple of
 * draw calls no matter how big the city gets.
 */
import * as THREE from "three";
import { TILE } from "./config";
import { cellCenterX, cellCenterZ } from "./grid";

const ASPHALT: [number, number, number] = [0.27, 0.29, 0.33];
const MARK: [number, number, number] = [0.92, 0.84, 0.42];
const SIDEWALK: [number, number, number] = [0.74, 0.76, 0.78];

const HALF = TILE / 2;
const ROAD_Y = 0.03;

interface Buf {
  pos: number[];
  col: number[];
}

/** Push a flat axis-aligned quad on the XZ plane (two triangles). */
function quad(
  buf: Buf,
  cx: number,
  cz: number,
  w: number,
  d: number,
  y: number,
  c: [number, number, number],
): void {
  const x0 = cx - w / 2;
  const x1 = cx + w / 2;
  const z0 = cz - d / 2;
  const z1 = cz + d / 2;
  const v = [x0, y, z0, x1, y, z0, x1, y, z1, x0, y, z0, x1, y, z1, x0, y, z1];
  buf.pos.push(...v);
  for (let i = 0; i < 6; i++) buf.col.push(c[0], c[1], c[2]);
}

/**
 * Build the merged road mesh. `isRoad(gx,gz)` reports whether a cell is
 * a road (for connection masks); `cells` is the road cell list.
 */
export function buildRoadMesh(
  cells: Array<{ gx: number; gz: number }>,
  isRoad: (gx: number, gz: number) => boolean,
): THREE.Mesh {
  const buf: Buf = { pos: [], col: [] };
  const laneW = TILE * 0.62; // paved width; rest is sidewalk
  const stripe = TILE * 0.05;

  for (const { gx, gz } of cells) {
    const cx = cellCenterX(gx);
    const cz = cellCenterZ(gz);
    // Sidewalk base (full cell), then asphalt pad on top.
    quad(buf, cx, cz, TILE, TILE, ROAD_Y, SIDEWALK);
    quad(buf, cx, cz, laneW, laneW, ROAD_Y + 0.01, ASPHALT);

    const n = isRoad(gx, gz + 1);
    const s = isRoad(gx, gz - 1);
    const e = isRoad(gx + 1, gz);
    const w = isRoad(gx - 1, gz);
    const my = ROAD_Y + 0.02;

    // Extend asphalt into each connected neighbour so the paved band is
    // continuous across the sidewalk gap.
    if (n) quad(buf, cx, cz + HALF * 0.75, laneW, TILE * 0.5, ROAD_Y + 0.01, ASPHALT);
    if (s) quad(buf, cx, cz - HALF * 0.75, laneW, TILE * 0.5, ROAD_Y + 0.01, ASPHALT);
    if (e) quad(buf, cx + HALF * 0.75, cz, TILE * 0.5, laneW, ROAD_Y + 0.01, ASPHALT);
    if (w) quad(buf, cx - HALF * 0.75, cz, TILE * 0.5, laneW, ROAD_Y + 0.01, ASPHALT);

    // Centre-line markings toward each connected direction.
    const conn = (n ? 1 : 0) + (s ? 1 : 0) + (e ? 1 : 0) + (w ? 1 : 0);
    if (conn >= 3) {
      // Intersection: small centre patch, no through-lines.
      quad(buf, cx, cz, stripe * 1.4, stripe * 1.4, my, MARK);
    } else {
      if (n) quad(buf, cx, cz + TILE * 0.28, stripe, TILE * 0.34, my, MARK);
      if (s) quad(buf, cx, cz - TILE * 0.28, stripe, TILE * 0.34, my, MARK);
      if (e) quad(buf, cx + TILE * 0.28, cz, TILE * 0.34, stripe, my, MARK);
      if (w) quad(buf, cx - TILE * 0.28, cz, TILE * 0.34, stripe, my, MARK);
      if (conn === 0) quad(buf, cx, cz, stripe * 1.2, stripe * 1.2, my, MARK);
    }
  }

  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(buf.pos, 3));
  geo.setAttribute("color", new THREE.Float32BufferAttribute(buf.col, 3));
  geo.computeVertexNormals();
  const mat = new THREE.MeshLambertMaterial({ vertexColors: true });
  const mesh = new THREE.Mesh(geo, mat);
  mesh.receiveShadow = true;
  mesh.name = "roads";
  return mesh;
}
