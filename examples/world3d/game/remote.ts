/**
 * Remote players. One animated character per Avatar row (see
 * character.ts — same model as the local third-person body), with the
 * synced pitch aiming the rifle and a canvas-sprite name tag.
 *
 * Server pose arrives at ~10 Hz; exponential smoothing interpolates
 * to 60 fps, and the interpolated velocity drives the walk cycle.
 * The synced y is the player's eye height — meshes ground themselves
 * against the terrain when close to it (so walkers don't float) but
 * keep the synced y while swimming or falling.
 */
import * as THREE from "three";
import type { FrameCtx, GameSystem } from "./engine";
import { PLAYER } from "./config";
import { buildCharacter, type Character } from "./character";
import type { Terrain } from "./terrain";

export interface AvatarRow {
  id: string;
  userId: string;
  name: string;
  color: string;
  x: number;
  y: number;
  z: number;
  heading: number;
  pitch: number;
  lastSeenAt: string;
}

interface RemoteEntry {
  character: Character;
  cur: { x: number; y: number; z: number; heading: number; pitch: number };
  target: { x: number; y: number; z: number; heading: number; pitch: number };
  disposables: Array<{ dispose(): void }>;
}

function makeNameSprite(name: string, color: string): { sprite: THREE.Sprite; disposables: Array<{ dispose(): void }> } {
  const canvas = document.createElement("canvas");
  canvas.width = 256;
  canvas.height = 64;
  const ctx = canvas.getContext("2d")!;
  ctx.font = "600 30px system-ui, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.shadowColor = "rgba(0,0,0,0.85)";
  ctx.shadowBlur = 6;
  ctx.fillStyle = color;
  ctx.fillText(name, 128, 32);

  const tex = new THREE.CanvasTexture(canvas);
  tex.colorSpace = THREE.SRGBColorSpace;
  const mat = new THREE.SpriteMaterial({ map: tex, depthWrite: false, sizeAttenuation: true });
  const sprite = new THREE.Sprite(mat);
  sprite.scale.set(2.6, 0.65, 1);
  return { sprite, disposables: [tex, mat] };
}

export class RemotePlayers implements GameSystem {
  readonly name = "remote-players";
  readonly group = new THREE.Group();

  private readonly entries = new Map<string, RemoteEntry>();
  private latest: AvatarRow[] = [];
  private selfUserId: string | null = null;

  constructor(private readonly terrain: Terrain) {
    this.group.name = "remote-players";
  }

  /** Kick a remote player's weapon (their shot just arrived). */
  fireCharacter(userId: string) {
    for (const row of this.latest) {
      if (row.userId !== userId) continue;
      this.entries.get(row.id)?.character.fire();
      return;
    }
  }

  /** Latest known pose for a user — anchors remote-fire VFX. */
  poseOf(userId: string): { x: number; y: number; z: number } | null {
    for (const row of this.latest) {
      if (row.userId !== userId) continue;
      const entry = this.entries.get(row.id);
      return entry ? { x: entry.cur.x, y: entry.cur.y, z: entry.cur.z } : { x: row.x, y: row.y, z: row.z };
    }
    return null;
  }

  /** Called from React whenever the Avatar live query updates. */
  setAvatars(rows: AvatarRow[], selfUserId: string | null) {
    this.latest = rows;
    this.selfUserId = selfUserId;
  }

  get count(): number {
    return this.entries.size;
  }

  /** Current interpolated positions for the minimap. */
  minimapDots(): Array<{ x: number; z: number; color: string }> {
    const dots: Array<{ x: number; z: number; color: string }> = [];
    for (const row of this.latest) {
      if (this.selfUserId && row.userId === this.selfUserId) continue;
      const entry = this.entries.get(row.id);
      dots.push({
        x: entry ? entry.cur.x : row.x,
        z: entry ? entry.cur.z : row.z,
        color: row.color,
      });
    }
    return dots;
  }

  private addEntry(row: AvatarRow): RemoteEntry {
    const character = buildCharacter(row.color);
    const disposables: Array<{ dispose(): void }> = [character];

    const tag = makeNameSprite(row.name, row.color);
    tag.sprite.position.y = 0.62;
    character.group.add(tag.sprite);
    disposables.push(...tag.disposables);

    character.group.position.set(row.x, row.y, row.z);
    this.group.add(character.group);

    const entry: RemoteEntry = {
      character,
      cur: { x: row.x, y: row.y, z: row.z, heading: row.heading, pitch: row.pitch },
      target: { x: row.x, y: row.y, z: row.z, heading: row.heading, pitch: row.pitch },
      disposables,
    };
    this.entries.set(row.id, entry);
    return entry;
  }

  private removeEntry(id: string) {
    const entry = this.entries.get(id);
    if (!entry) return;
    this.group.remove(entry.character.group);
    for (const d of entry.disposables) d.dispose();
    this.entries.delete(id);
  }

  update(ctx: FrameCtx) {
    const seen = new Set<string>();
    for (const row of this.latest) {
      if (this.selfUserId && row.userId === this.selfUserId) continue;
      seen.add(row.id);
      let entry = this.entries.get(row.id);
      if (!entry) entry = this.addEntry(row);
      // Coerce defensively: rows written by older schema versions may
      // miss fields, and one NaN would poison the whole matrix.
      entry.target.x = Number.isFinite(row.x) ? row.x : 0;
      entry.target.y = Number.isFinite(row.y) ? row.y : 0;
      entry.target.z = Number.isFinite(row.z) ? row.z : 0;
      entry.target.heading = Number.isFinite(row.heading) ? row.heading : 0;
      entry.target.pitch = Number.isFinite(row.pitch) ? row.pitch : 0;
    }
    for (const id of Array.from(this.entries.keys())) {
      if (!seen.has(id)) this.removeEntry(id);
    }

    // Interpolate toward targets.
    const lerp = 1 - Math.exp(-ctx.dt * 10);
    for (const entry of this.entries.values()) {
      const c = entry.cur;
      const t = entry.target;
      const prevX = c.x;
      const prevZ = c.z;
      c.x += (t.x - c.x) * lerp;
      c.z += (t.z - c.z) * lerp;

      // Ground the mesh when the synced eye-y is near the local
      // terrain (walking); trust the synced y otherwise (swim/fall).
      const groundEye = this.terrain.heightAt(c.x, c.z) + PLAYER.eyeHeight;
      const targetY = Math.abs(t.y - groundEye) < 1.2 ? groundEye : t.y;
      c.y += (targetY - c.y) * lerp;

      let dh = t.heading - c.heading;
      while (dh > Math.PI) dh -= Math.PI * 2;
      while (dh < -Math.PI) dh += Math.PI * 2;
      c.heading += dh * lerp;
      c.pitch += (t.pitch - c.pitch) * lerp;

      entry.character.group.position.set(c.x, c.y, c.z);
      entry.character.group.rotation.y = c.heading;
      entry.character.aimPivot.rotation.x = c.pitch;
      // Interpolated velocity drives the walk cycle.
      const speed = ctx.dt > 0 ? Math.hypot(c.x - prevX, c.z - prevZ) / ctx.dt : 0;
      entry.character.animate(ctx.time, speed);
    }
  }

  dispose() {
    for (const id of Array.from(this.entries.keys())) this.removeEntry(id);
    // Rig geometries/materials are module-shared singletons — they
    // stay alive for the local player's rig and future sessions.
  }
}
