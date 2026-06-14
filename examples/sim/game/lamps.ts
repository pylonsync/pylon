/**
 * Street lamps — warm lamp posts lining the road network. Static (rebuilt
 * only when the road set changes) and instanced, so the whole grid of lamps
 * costs ~2 draw calls. The heads carry a constant emissive: the day sun
 * washes it out, but as the day/night cycle's ambient falls they light the
 * streets and bloom, the way a Cities: Skylines night does. Cosmetic + local,
 * like traffic — not synced.
 */
import * as THREE from "three";
import { TILE } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import { cellCenterX, cellCenterZ } from "./grid";
import { heightAt } from "./terrain";

const POLE_H = 3.4;
const poleGeo = new THREE.CylinderGeometry(0.05, 0.07, POLE_H, 6).translate(0, POLE_H / 2, 0);
const headGeo = new THREE.SphereGeometry(0.2, 8, 6).translate(0, POLE_H + 0.05, 0);
const poleMat = new THREE.MeshStandardMaterial({
  color: 0x26282d,
  roughness: 0.7,
  metalness: 0.4,
});
const headMat = new THREE.MeshStandardMaterial({
  color: 0xfff0cc,
  emissive: 0xffcf7a,
  emissiveIntensity: 0.95,
  roughness: 0.5,
});

export class StreetLamps implements GameSystem {
  readonly name = "lamps";
  readonly group = new THREE.Group();
  private sig = "";
  private readonly m = new THREE.Matrix4();

  /** Rebuild the lamp instances for the current road network (cheap no-op
   *  if the roads are unchanged). */
  setRoads(
    roadCells: Array<{ gx: number; gz: number }>,
    isRoad: (gx: number, gz: number) => boolean,
  ): void {
    const sig = roadCells.map((c) => c.gx + ":" + c.gz).sort().join("|");
    if (sig === this.sig) return;
    this.sig = sig;
    this.clear();

    const inset = TILE * 0.38;
    const pts: Array<[number, number]> = [];
    for (const { gx, gz } of roadCells) {
      // Sparse + deterministic: roughly one lamp per three cells.
      let h = Math.abs(Math.sin(gx * 51.7 + gz * 19.3) * 7919.1);
      h -= Math.floor(h);
      if (h > 0.36) continue;
      // Seat it on the sidewalk of the first open (non-road) side; skip an
      // interior cell of an intersection (no open side).
      let ox = 0;
      let oz = 0;
      if (!isRoad(gx, gz + 1)) oz = inset;
      else if (!isRoad(gx, gz - 1)) oz = -inset;
      else if (!isRoad(gx + 1, gz)) ox = inset;
      else if (!isRoad(gx - 1, gz)) ox = -inset;
      else continue;
      pts.push([cellCenterX(gx) + ox, cellCenterZ(gz) + oz]);
    }
    if (pts.length === 0) return;

    const poles = new THREE.InstancedMesh(poleGeo, poleMat, pts.length);
    const heads = new THREE.InstancedMesh(headGeo, headMat, pts.length);
    poles.castShadow = true;
    for (let i = 0; i < pts.length; i++) {
      this.m.makeTranslation(pts[i][0], heightAt(pts[i][0], pts[i][1]), pts[i][1]);
      poles.setMatrixAt(i, this.m);
      heads.setMatrixAt(i, this.m);
    }
    poles.instanceMatrix.needsUpdate = true;
    heads.instanceMatrix.needsUpdate = true;
    this.group.add(poles);
    this.group.add(heads);
  }

  update(_ctx: FrameCtx): void {}

  private clear(): void {
    for (const c of this.group.children) (c as THREE.InstancedMesh).dispose?.();
    this.group.clear();
  }

  dispose(): void {
    this.clear();
  }
}
