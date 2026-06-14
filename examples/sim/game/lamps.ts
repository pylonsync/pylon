/**
 * Street lamps — warm lamp posts lining the road network. Static geometry
 * (rebuilt only when the road set changes) and instanced, so the whole grid of
 * posts/heads/glow-discs costs ~3 draw calls. Cosmetic + local, like traffic —
 * not synced.
 *
 * At night the lamps light the city two ways:
 *   - a cheap additive glow disc on the road under EVERY lamp (ground only), and
 *   - a small fixed pool of real PointLights that follow the lamps nearest the
 *     camera focus, so buildings, trees and people near you are actually lit in
 *     3-D (a flat decal can't light a wall). Hundreds of real lights would tank
 *     FPS; a handful that chase the view gives the look for a fixed cost.
 */
import * as THREE from "three";
import { TILE } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import { cellCenterX, cellCenterZ } from "./grid";
import { heightAt } from "./terrain";
import { makeLampPool } from "./textures";

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

// The glow each lamp paints on the road: a warm disc laid flat just above the
// asphalt, blended additively. A shared material whose opacity the day/night
// cycle drives (invisible by day, full at night). Ground only — see below.
const poolGeo = new THREE.CircleGeometry(TILE * 1.25, 20).rotateX(-Math.PI / 2);
const poolMat = new THREE.MeshBasicMaterial({
  map: makeLampPool(),
  transparent: true,
  blending: THREE.AdditiveBlending,
  depthWrite: false,
  opacity: 0,
});

// Real lights that chase the view. Fixed count → the material shaders compile
// once and never recompile (no dusk stutter); we only move them and ramp their
// intensity. Enough to cover a close street, few enough to stay cheap.
const LIGHT_COUNT = 14;
const LIGHT_RANGE = TILE * 4.5; // distance where the light reaches zero
const LIGHT_DECAY = 1.4; // a touch softer than physical 1/d² so a wall lights
const LIGHT_INTENSITY = 18; // scaled by the night factor

export class StreetLamps implements GameSystem {
  readonly name = "lamps";
  readonly group = new THREE.Group();
  private sig = "";
  private readonly m = new THREE.Matrix4();

  // Lamp positions as [x, groundY, z]; the head/light sit POLE_H above groundY.
  private lampPts: Array<[number, number, number]> = [];
  private readonly meshes: THREE.InstancedMesh[] = [];
  private readonly lights: THREE.PointLight[] = [];
  private night = 0;
  private readonly focus = new THREE.Vector3();
  // Scratch used to pick the nearest lamps each frame (index + distance²).
  private readonly order: Array<{ i: number; d: number }> = [];

  constructor() {
    for (let i = 0; i < LIGHT_COUNT; i++) {
      const L = new THREE.PointLight(0xffce86, 0, LIGHT_RANGE, LIGHT_DECAY);
      L.position.set(0, -1000, 0); // parked underground until assigned
      this.lights.push(L);
      this.group.add(L);
    }
  }

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
    this.lampPts = pts.map(([x, z]) => [x, heightAt(x, z), z]);
    if (pts.length === 0) return;

    const poles = new THREE.InstancedMesh(poleGeo, poleMat, pts.length);
    const heads = new THREE.InstancedMesh(headGeo, headMat, pts.length);
    const pools = new THREE.InstancedMesh(poolGeo, poolMat, pts.length);
    pools.renderOrder = 2; // draw the additive glow over the road surface
    poles.castShadow = true;
    for (let i = 0; i < pts.length; i++) {
      const y = this.lampPts[i][1];
      this.m.makeTranslation(pts[i][0], y, pts[i][1]);
      poles.setMatrixAt(i, this.m);
      heads.setMatrixAt(i, this.m);
      // The pool sits on the road just below the post, not at the head height.
      this.m.makeTranslation(pts[i][0], y + 0.07, pts[i][1]);
      pools.setMatrixAt(i, this.m);
    }
    for (const mesh of [poles, heads, pools]) {
      mesh.instanceMatrix.needsUpdate = true;
      this.meshes.push(mesh);
      this.group.add(mesh);
    }
  }

  /**
   * Drive the night lighting (0 day .. 1 night). Fades the ground glow discs
   * in and aims the real PointLights at the lamps nearest `focus` so the city
   * around the camera is genuinely lit, not just the road decals.
   */
  setNight(night: number, focus: THREE.Vector3): void {
    this.night = Math.max(0, Math.min(1, night));
    this.focus.copy(focus);
    poolMat.opacity = this.night * 0.9;
    this.aimLights();
  }

  /** Park the PointLights on the lamps nearest the focus, scaled by night. */
  private aimLights(): void {
    const pts = this.lampPts;
    if (this.night < 0.02 || pts.length === 0) {
      for (const L of this.lights) L.intensity = 0;
      return;
    }
    const fx = this.focus.x;
    const fz = this.focus.z;
    this.order.length = 0;
    for (let i = 0; i < pts.length; i++) {
      const dx = pts[i][0] - fx;
      const dz = pts[i][2] - fz;
      this.order.push({ i, d: dx * dx + dz * dz });
    }
    this.order.sort((a, b) => a.d - b.d);
    const intensity = this.night * LIGHT_INTENSITY;
    for (let k = 0; k < this.lights.length; k++) {
      const pick = this.order[k];
      const L = this.lights[k];
      if (!pick) {
        L.intensity = 0;
        continue;
      }
      const p = pts[pick.i];
      L.position.set(p[0], p[1] + POLE_H, p[2]);
      L.intensity = intensity;
    }
  }

  update(_ctx: FrameCtx): void {}

  private clear(): void {
    for (const mesh of this.meshes) {
      this.group.remove(mesh);
      mesh.dispose();
    }
    this.meshes.length = 0;
  }

  dispose(): void {
    this.clear();
    for (const L of this.lights) this.group.remove(L);
    this.lights.length = 0;
  }
}
