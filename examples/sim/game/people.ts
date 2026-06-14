/**
 * Pedestrians — a crowd of low-poly figures walking the sidewalks. Local +
 * cosmetic (not synced), like traffic: each client runs its own crowd over
 * the shared road graph. One InstancedMesh, so the whole crowd is a single
 * draw call; figures walk the road network on the kerb offset, slower than
 * cars, with a gentle walking bob.
 */
import * as THREE from "three";
import { TILE } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import { cellCenterX, cellCenterZ, cellKey } from "./grid";
import { heightAt } from "./terrain";

const SIDEWALK = TILE * 0.42; // offset from the road centre onto the kerb

// A body capsule + a head sphere (~1.0 unit ≈ 1.7 m tall), base at y=0.
const FIGURE_GEO = new THREE.CapsuleGeometry(0.15, 0.5, 3, 6).translate(0, 0.4, 0);
const HEAD_GEO = new THREE.SphereGeometry(0.16, 6, 5).translate(0, 0.88, 0);
const FIGURE_MAT = new THREE.MeshStandardMaterial({ roughness: 0.85, flatShading: true });
const HEAD_MAT = new THREE.MeshStandardMaterial({ roughness: 0.9, flatShading: true });
const CLOTHES = [
  0x3a5a8c, 0x8c3a3a, 0x44464d, 0x6b8c3a, 0xb0792a, 0x7a3a6b, 0xcfcfcf, 0x2a6b6b,
];
const SKIN = [0xe8c39a, 0xcf9a6e, 0x9b6a44, 0x6b4a32, 0xf0d2b0];

interface Walker {
  from: string;
  to: string;
  t: number;
  speed: number;
  side: number; // which kerb (+1 / -1)
  phase: number; // bob offset
  scale: number; // height variation
}

export class People implements GameSystem {
  readonly name = "people";
  readonly group = new THREE.Group();

  private graph = new Map<string, string[]>();
  private walkers: Walker[] = [];
  private mesh: THREE.InstancedMesh | null = null;
  private headMesh: THREE.InstancedMesh | null = null;
  private sig = "";
  private readonly dummy = new THREE.Object3D();
  private readonly tmpColor = new THREE.Color();

  /** Rebuild the walk graph + crowd when the road set changes. */
  setRoads(
    roadCells: Array<{ gx: number; gz: number }>,
    isRoad: (gx: number, gz: number) => boolean,
  ): void {
    const sig = roadCells.map((c) => c.gx + ":" + c.gz).sort().join("|");
    if (sig === this.sig) return;
    this.sig = sig;

    this.graph.clear();
    for (const { gx, gz } of roadCells) {
      const nb: string[] = [];
      if (isRoad(gx + 1, gz)) nb.push(cellKey(gx + 1, gz));
      if (isRoad(gx - 1, gz)) nb.push(cellKey(gx - 1, gz));
      if (isRoad(gx, gz + 1)) nb.push(cellKey(gx, gz + 1));
      if (isRoad(gx, gz - 1)) nb.push(cellKey(gx, gz - 1));
      if (nb.length) this.graph.set(cellKey(gx, gz), nb);
    }

    const target = Math.min(170, Math.floor(this.graph.size * 1.4));
    this.walkers = [];
    for (let i = 0; i < target; i++) {
      const w = this.spawn();
      if (w) this.walkers.push(w);
    }

    this.clearMeshes();
    if (this.walkers.length === 0) return;
    const n = this.walkers.length;
    const mesh = new THREE.InstancedMesh(FIGURE_GEO, FIGURE_MAT, n);
    const heads = new THREE.InstancedMesh(HEAD_GEO, HEAD_MAT, n);
    mesh.castShadow = true;
    heads.castShadow = true;
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    heads.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    for (let i = 0; i < n; i++) {
      this.tmpColor.setHex(CLOTHES[(Math.random() * CLOTHES.length) | 0]);
      mesh.setColorAt(i, this.tmpColor);
      this.tmpColor.setHex(SKIN[(Math.random() * SKIN.length) | 0]);
      heads.setColorAt(i, this.tmpColor);
    }
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    if (heads.instanceColor) heads.instanceColor.needsUpdate = true;
    this.mesh = mesh;
    this.headMesh = heads;
    this.group.add(mesh, heads);
  }

  private clearMeshes(): void {
    for (const m of [this.mesh, this.headMesh]) {
      if (m) {
        this.group.remove(m);
        m.dispose();
      }
    }
    this.mesh = null;
    this.headMesh = null;
  }

  private spawn(): Walker | null {
    const keys = [...this.graph.keys()];
    if (keys.length === 0) return null;
    const from = keys[(Math.random() * keys.length) | 0];
    const nb = this.graph.get(from);
    if (!nb || nb.length === 0) return null;
    return {
      from,
      to: nb[(Math.random() * nb.length) | 0],
      t: Math.random(),
      speed: 1.1 + Math.random() * 0.9,
      side: Math.random() < 0.5 ? 1 : -1,
      phase: Math.random() * Math.PI * 2,
      scale: 0.86 + Math.random() * 0.24,
    };
  }

  private next(from: string, to: string): string {
    const nb = this.graph.get(to);
    if (!nb || nb.length === 0) return from;
    const fwd = nb.filter((k) => k !== from);
    const pool = fwd.length ? fwd : nb; // dead end → turn back
    return pool[(Math.random() * pool.length) | 0];
  }

  update(ctx: FrameCtx): void {
    const mesh = this.mesh;
    const heads = this.headMesh;
    if (!mesh || !heads || this.walkers.length === 0) return;
    for (let i = 0; i < this.walkers.length; i++) {
      const w = this.walkers[i];
      w.t += (w.speed * ctx.dt) / TILE;
      while (w.t >= 1) {
        w.t -= 1;
        const nf = w.to;
        w.to = this.next(w.from, w.to);
        w.from = nf;
      }
      const [fgx, fgz] = w.from.split(",").map(Number);
      const [tgx, tgz] = w.to.split(",").map(Number);
      const ax = cellCenterX(fgx);
      const az = cellCenterZ(fgz);
      const dx = cellCenterX(tgx) - ax;
      const dz = cellCenterZ(tgz) - az;
      const len = Math.hypot(dx, dz) || 1;
      const ux = dx / len;
      const uz = dz / len;
      // Walk on the kerb: offset perpendicular to the right/left of travel.
      const px = ax + dx * w.t + -uz * SIDEWALK * w.side;
      const pz = az + dz * w.t + ux * SIDEWALK * w.side;
      const bob = Math.abs(Math.sin(ctx.time * 6 + w.phase)) * 0.06;
      this.dummy.position.set(px, heightAt(px, pz) + 0.12 + bob, pz);
      this.dummy.rotation.set(0, Math.atan2(ux, uz), 0);
      this.dummy.scale.setScalar(w.scale);
      this.dummy.updateMatrix();
      mesh.setMatrixAt(i, this.dummy.matrix);
      heads.setMatrixAt(i, this.dummy.matrix); // head rides the body transform
    }
    mesh.instanceMatrix.needsUpdate = true;
    heads.instanceMatrix.needsUpdate = true;
  }

  dispose(): void {
    this.clearMeshes();
    this.group.clear();
  }
}
