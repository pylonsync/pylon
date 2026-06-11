/**
 * Rigid debris pool. When a building block is destroyed it becomes a
 * tumbling chunk: integrated with gravity, bounced off the terrain
 * heightfield, and shrunk out at end of life. One InstancedMesh, one
 * draw call, fixed capacity — saturation drops the oldest pieces.
 */
import * as THREE from "three";
import { BLOCK_SIZE, GRAVITY } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import type { Terrain } from "./terrain";

const MAX_DEBRIS = 320;

interface Piece {
  active: boolean;
  pos: THREE.Vector3;
  vel: THREE.Vector3;
  rot: THREE.Euler;
  angVel: THREE.Vector3;
  scale: number;
  life: number;
  maxLife: number;
}

export class Debris implements GameSystem {
  readonly name = "debris";
  readonly mesh: THREE.InstancedMesh;

  private readonly pieces: Piece[] = [];
  private cursor = 0;
  private readonly m = new THREE.Matrix4();
  private readonly q = new THREE.Quaternion();
  private readonly s = new THREE.Vector3();
  private readonly hidden = new THREE.Matrix4().makeScale(0, 0, 0);

  constructor(
    private readonly terrain: Terrain,
    concreteMap?: THREE.Texture,
  ) {
    const geo = new THREE.BoxGeometry(BLOCK_SIZE * 0.55, BLOCK_SIZE * 0.55, BLOCK_SIZE * 0.55);
    const mat = new THREE.MeshStandardMaterial({ roughness: 0.9, map: concreteMap ?? null });
    this.mesh = new THREE.InstancedMesh(geo, mat, MAX_DEBRIS);
    this.mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    this.mesh.frustumCulled = false;
    // Per-instance color so chunks inherit their block's tint.
    this.mesh.instanceColor = new THREE.InstancedBufferAttribute(
      new Float32Array(MAX_DEBRIS * 3).fill(1),
      3,
    );

    for (let i = 0; i < MAX_DEBRIS; i++) {
      this.pieces.push({
        active: false,
        pos: new THREE.Vector3(),
        vel: new THREE.Vector3(),
        rot: new THREE.Euler(),
        angVel: new THREE.Vector3(),
        scale: 1,
        life: 0,
        maxLife: 1,
      });
      this.mesh.setMatrixAt(i, this.hidden);
    }
    this.mesh.instanceMatrix.needsUpdate = true;
  }

  /**
   * Launch debris chunks from a destroyed block.
   * `impulse` points away from the cause (shot direction / blast center).
   */
  spawnFromBlock(center: THREE.Vector3, color: THREE.Color, impulse: THREE.Vector3, count = 3) {
    for (let n = 0; n < count; n++) {
      const i = this.cursor;
      this.cursor = (this.cursor + 1) % MAX_DEBRIS;
      const p = this.pieces[i];
      p.active = true;
      p.pos.copy(center).add(
        new THREE.Vector3(
          (Math.random() - 0.5) * BLOCK_SIZE,
          (Math.random() - 0.5) * BLOCK_SIZE,
          (Math.random() - 0.5) * BLOCK_SIZE,
        ),
      );
      p.vel
        .copy(impulse)
        .multiplyScalar(0.5 + Math.random())
        .add(new THREE.Vector3((Math.random() - 0.5) * 4, Math.random() * 5 + 2, (Math.random() - 0.5) * 4));
      p.rot.set(Math.random() * Math.PI, Math.random() * Math.PI, Math.random() * Math.PI);
      p.angVel.set((Math.random() - 0.5) * 10, (Math.random() - 0.5) * 10, (Math.random() - 0.5) * 10);
      p.scale = 0.6 + Math.random() * 0.8;
      p.maxLife = 2.5 + Math.random() * 2;
      p.life = p.maxLife;
      this.mesh.instanceColor!.setXYZ(i, color.r, color.g, color.b);
    }
    this.mesh.instanceColor!.needsUpdate = true;
  }

  update(ctx: FrameCtx) {
    const dt = ctx.dt;
    let any = false;
    for (let i = 0; i < MAX_DEBRIS; i++) {
      const p = this.pieces[i];
      if (!p.active) continue;
      any = true;
      p.life -= dt;
      if (p.life <= 0) {
        p.active = false;
        this.mesh.setMatrixAt(i, this.hidden);
        continue;
      }

      p.vel.y -= GRAVITY * dt;
      p.pos.addScaledVector(p.vel, dt);
      p.rot.x += p.angVel.x * dt;
      p.rot.y += p.angVel.y * dt;
      p.rot.z += p.angVel.z * dt;

      // Terrain bounce with energy loss; settle when slow.
      const ground = this.terrain.heightAt(p.pos.x, p.pos.z) + BLOCK_SIZE * 0.3;
      if (p.pos.y < ground) {
        p.pos.y = ground;
        if (Math.abs(p.vel.y) > 1.5) {
          p.vel.y = -p.vel.y * 0.35;
          p.vel.x *= 0.6;
          p.vel.z *= 0.6;
          p.angVel.multiplyScalar(0.6);
        } else {
          p.vel.set(0, 0, 0);
          p.angVel.set(0, 0, 0);
        }
      }

      // Shrink out over the last 25% of life.
      const lifeT = p.life / p.maxLife;
      const scale = p.scale * (lifeT < 0.25 ? lifeT / 0.25 : 1);

      this.q.setFromEuler(p.rot);
      this.s.setScalar(scale);
      this.m.compose(p.pos, this.q, this.s);
      this.mesh.setMatrixAt(i, this.m);
    }
    if (any) this.mesh.instanceMatrix.needsUpdate = true;
  }

  dispose() {
    this.mesh.geometry.dispose();
    (this.mesh.material as THREE.Material).dispose();
    this.mesh.dispose();
  }
}
