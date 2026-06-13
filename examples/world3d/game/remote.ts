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
import type { EventBus, FrameCtx, GameSystem } from "./engine";
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
  health?: number | null;
  lastSeenAt: string;
}

interface RemoteEntry {
  character: Character;
  /** Carries position/heading; the character tips over inside it on
   *  death while the name tag + health bar sprites stay upright. */
  root: THREE.Group;
  cur: { x: number; y: number; z: number; heading: number; pitch: number };
  target: { x: number; y: number; z: number; heading: number; pitch: number };
  healthBar: HealthBar;
  /** Last synced health — a drop between frames triggers blood VFX. */
  lastHealth: number;
  /** 0 = standing, 1 = prone; eases in on death, snaps on respawn. */
  deathT: number;
  disposables: Array<{ dispose(): void }>;
}

/** Hit info from a ray-vs-player capsule test. */
export interface PlayerHit {
  avatarId: string;
  userId: string;
  point: THREE.Vector3;
  dist: number;
}

/** Canvas-sprite health bar floating under the name tag. */
class HealthBar {
  readonly sprite: THREE.Sprite;
  private readonly canvas: HTMLCanvasElement;
  private readonly tex: THREE.CanvasTexture;
  private readonly mat: THREE.SpriteMaterial;
  private shown = -1;

  constructor() {
    this.canvas = document.createElement("canvas");
    this.canvas.width = 96;
    this.canvas.height = 12;
    this.tex = new THREE.CanvasTexture(this.canvas);
    this.mat = new THREE.SpriteMaterial({ map: this.tex, depthWrite: false });
    this.sprite = new THREE.Sprite(this.mat);
    this.sprite.scale.set(0.9, 0.11, 1);
    this.set(100);
  }

  /** Redraws only when the displayed value actually changes. */
  set(health: number) {
    const h = Math.round(Math.max(0, Math.min(100, health)));
    if (h === this.shown) return;
    this.shown = h;
    const ctx = this.canvas.getContext("2d")!;
    ctx.clearRect(0, 0, 96, 12);
    ctx.fillStyle = "rgba(0,0,0,0.55)";
    ctx.fillRect(0, 0, 96, 12);
    const pct = h / 100;
    // Green → amber → red as health drains.
    ctx.fillStyle = pct > 0.5 ? "#4ade5b" : pct > 0.25 ? "#eab308" : "#ef4444";
    ctx.fillRect(2, 2, 92 * pct, 8);
    this.tex.needsUpdate = true;
  }

  dispose() {
    this.tex.dispose();
    this.mat.dispose();
  }
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

  constructor(
    private readonly terrain: Terrain,
    private readonly events: EventBus,
  ) {
    this.group.name = "remote-players";
  }

  /** World position of a remote player's gun muzzle (tracer anchor). */
  muzzleWorldOf(userId: string): THREE.Vector3 | null {
    for (const row of this.latest) {
      if (row.userId !== userId) continue;
      const entry = this.entries.get(row.id);
      if (!entry) return null;
      return entry.character.muzzle.getWorldPosition(new THREE.Vector3());
    }
    return null;
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
  minimapDots(): Array<{ x: number; z: number }> {
    const dots: Array<{ x: number; z: number }> = [];
    for (const row of this.latest) {
      if (this.selfUserId && row.userId === this.selfUserId) continue;
      const entry = this.entries.get(row.id);
      dots.push({
        x: entry ? entry.cur.x : row.x,
        z: entry ? entry.cur.z : row.z,
      });
    }
    return dots;
  }

  private addEntry(row: AvatarRow): RemoteEntry {
    const character = buildCharacter(row.color);
    const disposables: Array<{ dispose(): void }> = [character];

    const root = new THREE.Group();
    root.add(character.group);

    const tag = makeNameSprite(row.name, row.color);
    tag.sprite.position.y = 0.62;
    root.add(tag.sprite);
    disposables.push(...tag.disposables);

    const healthBar = new HealthBar();
    healthBar.sprite.position.y = 0.42;
    root.add(healthBar.sprite);
    disposables.push(healthBar);

    root.position.set(row.x, row.y, row.z);
    this.group.add(root);

    const entry: RemoteEntry = {
      character,
      root,
      cur: { x: row.x, y: row.y, z: row.z, heading: row.heading, pitch: row.pitch },
      target: { x: row.x, y: row.y, z: row.z, heading: row.heading, pitch: row.pitch },
      healthBar,
      // Seed from the row so a player FIRST SEEN at low health doesn't
      // splatter on arrival.
      lastHealth: row.health ?? 100,
      deathT: (row.health ?? 100) <= 0 ? 1 : 0,
      disposables,
    };
    this.entries.set(row.id, entry);
    return entry;
  }

  /**
   * Ray vs every remote player's body capsule (eye at cur.y, soles
   * ~1.62 below, radius 0.5 for the chunky build). Returns the nearest
   * hit or null. Closest-approach math on the capsule's vertical
   * segment — cheap enough to run per shot for dozens of players.
   */
  raycastPlayers(origin: THREE.Vector3, dir: THREE.Vector3, far: number): PlayerHit | null {
    let best: PlayerHit | null = null;
    const RADIUS = 0.5;
    for (const [id, entry] of this.entries) {
      if (entry.lastHealth <= 0) continue; // corpses don't soak bullets
      const cx = entry.cur.x;
      const cz = entry.cur.z;
      const yLo = entry.cur.y - 1.62;
      const yHi = entry.cur.y + 0.15;

      // Parametrize: ray P(t) = O + tD; capsule axis Q(s) = (cx, yLo+s, cz).
      // Solve closest approach between the two lines, clamp s to the
      // segment, then check perpendicular distance.
      const ox = origin.x - cx;
      const oz = origin.z - cz;
      // Horizontal quadratic: |(ox + t*dx, oz + t*dz)| = RADIUS.
      const a = dir.x * dir.x + dir.z * dir.z;
      const b = 2 * (ox * dir.x + oz * dir.z);
      const c = ox * ox + oz * oz - RADIUS * RADIUS;
      let t: number;
      if (a < 1e-6) {
        // Ray is vertical — inside the cylinder column or not.
        if (c > 0) continue;
        t = 0;
      } else {
        const disc = b * b - 4 * a * c;
        if (disc < 0) continue;
        t = (-b - Math.sqrt(disc)) / (2 * a);
        if (t < 0 || t > far) continue;
      }
      const hitY = origin.y + dir.y * t;
      if (hitY < yLo || hitY > yHi + RADIUS) continue;
      if (!best || t < best.dist) {
        const row = this.latest.find((r) => r.id === id);
        best = {
          avatarId: id,
          userId: row?.userId ?? "",
          point: new THREE.Vector3(origin.x + dir.x * t, hitY, origin.z + dir.z * t),
          dist: t,
        };
      }
    }
    return best;
  }

  /** Remote players within `radius` of a point (grenade splash). */
  playersInRadius(center: THREE.Vector3, radius: number): Array<{ avatarId: string; dist: number }> {
    const out: Array<{ avatarId: string; dist: number }> = [];
    for (const [id, entry] of this.entries) {
      if (entry.lastHealth <= 0) continue; // already down — no splash
      const d = Math.hypot(
        entry.cur.x - center.x,
        entry.cur.y - 0.8 - center.y, // body center, not eye
        entry.cur.z - center.z,
      );
      if (d <= radius) out.push({ avatarId: id, dist: d });
    }
    return out;
  }

  private removeEntry(id: string) {
    const entry = this.entries.get(id);
    if (!entry) return;
    this.group.remove(entry.root);
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
    for (const [id, entry] of this.entries) {
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

      entry.root.position.set(c.x, c.y, c.z);
      entry.root.rotation.y = c.heading;
      entry.character.aimPivot.rotation.x = c.pitch;
      // Interpolated velocity drives the walk cycle.
      const speed = ctx.dt > 0 ? Math.hypot(c.x - prevX, c.z - prevZ) / ctx.dt : 0;
      entry.character.animate(ctx.time, speed);
      // Health bar follows the synced row; a drop means this player
      // just took a hit — splatter at chest height so EVERY client
      // sees the gore, not just the shooter.
      const row = this.latest.find((r) => r.id === id);
      if (row) {
        const health = row.health ?? 100;
        if (health < entry.lastHealth) {
          this.events.emit("blood", {
            point: new THREE.Vector3(c.x, c.y - 0.55, c.z),
          });
        }
        entry.lastHealth = health;
        entry.healthBar.set(health);
      }
      // Death pose: ease the body backward to the ground at 0 HP;
      // snap upright on respawn (the row teleports anyway). The tip
      // happens INSIDE root so the tag/bar sprites stay overhead.
      if (entry.lastHealth <= 0) {
        entry.deathT += (1 - entry.deathT) * Math.min(1, ctx.dt * 5);
      } else {
        entry.deathT = 0;
      }
      entry.character.group.rotation.x = entry.deathT * 1.35;
      entry.character.group.position.y = -entry.deathT * 1.2;
    }
  }

  dispose() {
    for (const id of Array.from(this.entries.keys())) this.removeEntry(id);
    // Rig geometries/materials are module-shared singletons — they
    // stay alive for the local player's rig and future sessions.
  }
}
