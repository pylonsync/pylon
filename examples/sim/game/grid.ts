/**
 * Grid coordinate math + the ground plane and hover cursor meshes.
 * Cell (0,0) is the south-west corner; the grid is centred on the
 * world origin.
 */
import * as THREE from "three";
import { GRID, TILE, WORLD_HALF } from "./config";
import { makeGroundTexture } from "./textures";

export function cellCenterX(gx: number): number {
  return (gx - GRID / 2 + 0.5) * TILE;
}
export function cellCenterZ(gz: number): number {
  return (gz - GRID / 2 + 0.5) * TILE;
}

export function worldToCell(x: number, z: number): { gx: number; gz: number } {
  return {
    gx: Math.floor(x / TILE + GRID / 2),
    gz: Math.floor(z / TILE + GRID / 2),
  };
}

export function inBounds(gx: number, gz: number): boolean {
  return gx >= 0 && gz >= 0 && gx < GRID && gz < GRID;
}

export function cellKey(gx: number, gz: number): string {
  return gx + "," + gz;
}

/** Big ground plane with a faint repeating grid texture. */
export function makeGround(): THREE.Mesh {
  const tex = makeGroundTexture();
  tex.repeat.set(GRID, GRID);
  const geo = new THREE.PlaneGeometry(GRID * TILE, GRID * TILE);
  geo.rotateX(-Math.PI / 2);
  const mat = new THREE.MeshLambertMaterial({ map: tex, color: 0xffffff });
  const mesh = new THREE.Mesh(geo, mat);
  mesh.receiveShadow = true;
  mesh.name = "ground";
  return mesh;
}

/** Border frame so the buildable area is obvious. */
export function makeBorder(): THREE.LineSegments {
  const h = 0.05;
  const e = WORLD_HALF;
  const pts = [
    -e, h, -e, e, h, -e,
    e, h, -e, e, h, e,
    e, h, e, -e, h, e,
    -e, h, e, -e, h, -e,
  ];
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(pts, 3));
  return new THREE.LineSegments(geo, new THREE.LineBasicMaterial({ color: 0x3b4a59 }));
}

/** A 1×1-cell highlight quad that snaps to the hovered cell. */
export function makeCursor(): THREE.Mesh {
  const geo = new THREE.PlaneGeometry(TILE, TILE);
  geo.rotateX(-Math.PI / 2);
  const mat = new THREE.MeshBasicMaterial({
    color: 0xffffff,
    transparent: true,
    opacity: 0.28,
    depthWrite: false,
  });
  const mesh = new THREE.Mesh(geo, mat);
  mesh.position.y = 0.04;
  mesh.visible = false;
  mesh.renderOrder = 2;
  return mesh;
}
