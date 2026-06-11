/**
 * Rifle + grenades.
 *
 * The rifle is hitscan: ray from the camera against buildings first,
 * then terrain (coarse march), then water plane. Hits emit events the
 * other systems react to (particles, debris, building destruction,
 * network sync). The viewmodel is a small procedural mesh group
 * parented to the camera with recoil/reload animation driven in code.
 *
 * Grenades (G / right-click) are simulated projectiles that detonate
 * on contact and carve a sphere out of any building in range.
 */
import * as THREE from "three";
import type { Buildings } from "./buildings";
import { WATER_LEVEL, WEAPON } from "./config";
import type { EventBus, FrameCtx, GameSystem } from "./engine";
import type { Player } from "./player";
import type { Terrain } from "./terrain";

interface Grenade {
  mesh: THREE.Mesh;
  vel: THREE.Vector3;
  ttl: number;
  active: boolean;
  /** Remote grenades are VFX-only: their destruction arrives as
   *  Destruction rows from the thrower, never from this client. */
  remote: boolean;
}

export interface WeaponState {
  ammo: number;
  reloading: boolean;
}

const RIFLE_DAMAGE = 18;
const GRENADE_MAX_DAMAGE = 55;

/** Hooks into the remote-player system (wired after construction —
 *  RemotePlayers is built later in the composition order). */
export interface PlayerTargets {
  raycast(origin: THREE.Vector3, dir: THREE.Vector3, far: number):
    { avatarId: string; point: THREE.Vector3; dist: number } | null;
  inRadius(center: THREE.Vector3, radius: number): Array<{ avatarId: string; dist: number }>;
}

export class Weapon implements GameSystem {
  readonly name = "weapon";
  readonly viewmodel = new THREE.Group();

  ammo: number = WEAPON.magSize;
  reloading = false;
  /** Set by the game once RemotePlayers exists; null = no PvP targets. */
  playerTargets: PlayerTargets | null = null;
  /** Death gate — a dead player can't fire or lob grenades. */
  enabled = true;

  private firing = false;
  private lastShotAt = -Infinity;
  private reloadEndsAt = 0;
  private lastGrenadeAt = -Infinity;
  private viewKick = 0;

  private readonly grenades: Grenade[] = [];
  private readonly muzzleFlash: THREE.PointLight;
  private flashTtl = 0;

  private readonly tmpDir = new THREE.Vector3();
  private readonly tmpOrigin = new THREE.Vector3();
  private readonly tmpPoint = new THREE.Vector3();
  private readonly tmpNormal = new THREE.Vector3();

  private readonly onMouseDown = (e: MouseEvent) => {
    if (document.pointerLockElement === null) return;
    if (e.button === 0) this.firing = true;
    if (e.button === 2) this.queueGrenade = true;
  };
  private readonly onMouseUp = (e: MouseEvent) => {
    if (e.button === 0) this.firing = false;
  };
  private readonly onKeyDown = (e: KeyboardEvent) => {
    if (document.pointerLockElement === null) return;
    if (e.code === "KeyR") this.startReload();
    if (e.code === "KeyG") this.queueGrenade = true;
  };
  private readonly onContextMenu = (e: Event) => e.preventDefault();

  private queueGrenade = false;
  private timeNow = 0;

  constructor(
    private readonly scene: THREE.Scene,
    private readonly camera: THREE.PerspectiveCamera,
    private readonly player: Player,
    private readonly terrain: Terrain,
    private readonly buildings: Buildings,
    private readonly events: EventBus,
  ) {
    this.buildViewmodel();
    camera.add(this.viewmodel);

    this.muzzleFlash = new THREE.PointLight("#ffc066", 0, 14, 2);
    this.viewmodel.add(this.muzzleFlash);
    this.muzzleFlash.position.set(0, 0.02, -0.55);

    // Grenade pool.
    const gGeo = new THREE.SphereGeometry(0.09, 10, 8);
    const gMat = new THREE.MeshStandardMaterial({ color: "#3a4a38", roughness: 0.6 });
    for (let i = 0; i < 8; i++) {
      const mesh = new THREE.Mesh(gGeo, gMat);
      mesh.visible = false;
      scene.add(mesh);
      this.grenades.push({ mesh, vel: new THREE.Vector3(), ttl: 0, active: false, remote: false });
    }

    window.addEventListener("mousedown", this.onMouseDown);
    window.addEventListener("mouseup", this.onMouseUp);
    window.addEventListener("keydown", this.onKeyDown);
    window.addEventListener("contextmenu", this.onContextMenu);
  }

  get state(): WeaponState {
    return { ammo: this.ammo, reloading: this.reloading };
  }

  /** Simple boxy rifle — reads as a weapon without any assets. */
  private buildViewmodel() {
    // Slight emissive floor keeps the gun readable when its camera-
    // facing side is away from the sun (a pure-black wedge otherwise).
    const metal = (color: string, roughness: number) =>
      new THREE.MeshStandardMaterial({
        color,
        roughness,
        metalness: 0.35,
        emissive: color,
        emissiveIntensity: 0.35,
      });
    const body = new THREE.Mesh(new THREE.BoxGeometry(0.05, 0.075, 0.42), metal("#3a3e44", 0.55));
    const barrel = new THREE.Mesh(
      new THREE.CylinderGeometry(0.016, 0.016, 0.3, 8),
      metal("#272a2e", 0.45),
    );
    barrel.rotation.x = Math.PI / 2;
    barrel.position.set(0, 0.015, -0.32);
    const grip = new THREE.Mesh(new THREE.BoxGeometry(0.038, 0.12, 0.05), metal("#2e3136", 0.7));
    grip.position.set(0, -0.09, 0.06);
    grip.rotation.x = 0.3;
    const mag = new THREE.Mesh(new THREE.BoxGeometry(0.034, 0.1, 0.045), metal("#43474d", 0.6));
    mag.position.set(0, -0.085, -0.06);
    mag.rotation.x = -0.15;
    const sight = new THREE.Mesh(new THREE.BoxGeometry(0.018, 0.03, 0.07), metal("#26282c", 0.5));
    sight.position.set(0, 0.052, -0.05);

    this.viewmodel.add(body, barrel, grip, mag, sight);
    this.viewmodel.position.set(0.24, -0.2, -0.5);
    this.viewmodel.rotation.y = 0.05;
  }

  private startReload() {
    if (this.reloading || this.ammo === WEAPON.magSize) return;
    this.reloading = true;
    this.reloadEndsAt = this.timeNow + WEAPON.reloadMs / 1000;
  }

  private fire(ctx: FrameCtx) {
    this.lastShotAt = ctx.time;
    this.ammo--;
    this.viewKick = 1;
    this.flashTtl = 0.05;
    this.player.kick(0.011);

    this.camera.getWorldDirection(this.tmpDir);
    this.tmpOrigin.setFromMatrixPosition(this.camera.matrixWorld);

    // Nearest of: player capsule, building block, terrain.
    const playerHit = this.playerTargets?.raycast(this.tmpOrigin, this.tmpDir, WEAPON.range) ?? null;
    const blockHit = this.buildings.raycastBlocks(this.tmpOrigin, this.tmpDir, WEAPON.range);
    const terrainDist = this.terrain.raycast(this.tmpOrigin, this.tmpDir, WEAPON.range);
    const blockDist = blockHit ? blockHit.center.distanceTo(this.tmpOrigin) : Infinity;
    const playerDist = playerHit?.dist ?? Infinity;

    // The shot event carries the TRUE hit distance so tracers stop at
    // the impact point instead of streaking to max range.
    const hitDist = Math.min(blockDist, terrainDist ?? Infinity, playerDist);
    this.events.emit("shot", {
      origin: this.tmpOrigin,
      direction: this.tmpDir,
      dist: Number.isFinite(hitDist) ? hitDist : WEAPON.range,
    });

    if (playerHit && playerDist <= Math.min(blockDist, terrainDist ?? Infinity)) {
      this.events.emit("playerHit", {
        avatarId: playerHit.avatarId,
        point: playerHit.point,
        damage: RIFLE_DAMAGE,
      });
      return;
    }

    if (blockHit && blockDist <= (terrainDist ?? Infinity)) {
      this.events.emit("blocksDestroyed", { keys: [blockHit.key] });
      this.events.emit("impact", {
        point: blockHit.center,
        normal: this.tmpNormal.copy(this.tmpDir).negate(),
        surface: "building",
      });
    } else if (terrainDist !== null) {
      this.tmpPoint.copy(this.tmpOrigin).addScaledVector(this.tmpDir, terrainDist);
      const surface = this.tmpPoint.y < WATER_LEVEL + 0.3 ? "water" : "terrain";
      if (surface === "water") this.tmpPoint.y = WATER_LEVEL;
      this.events.emit("impact", {
        point: this.tmpPoint,
        normal: this.terrain.normalAt(this.tmpPoint.x, this.tmpPoint.z, this.tmpNormal),
        surface,
      });
    }
  }


  private throwGrenade(ctx: FrameCtx) {
    const g = this.grenades.find((x) => !x.active);
    if (!g) return;
    this.lastGrenadeAt = ctx.time;
    this.camera.getWorldDirection(this.tmpDir);
    this.tmpOrigin.setFromMatrixPosition(this.camera.matrixWorld);
    g.mesh.position.copy(this.tmpOrigin).addScaledVector(this.tmpDir, 0.6);
    g.vel.copy(this.tmpDir).multiplyScalar(WEAPON.grenadeSpeed);
    g.vel.y += 3.5; // slight lob
    g.ttl = 4;
    g.active = true;
    g.remote = false;
    g.mesh.visible = true;
    this.events.emit("grenadeThrown", {
      origin: g.mesh.position.clone(),
      velocity: g.vel.clone(),
    });
  }

  /** Replicate a peer's grenade as a visual-only projectile. */
  spawnRemoteGrenade(origin: THREE.Vector3, velocity: THREE.Vector3) {
    const g = this.grenades.find((x) => !x.active);
    if (!g) return;
    g.mesh.position.copy(origin);
    g.vel.copy(velocity);
    g.ttl = 4;
    g.active = true;
    g.remote = true;
    g.mesh.visible = true;
  }

  private detonate(g: Grenade) {
    g.active = false;
    g.mesh.visible = false;
    const point = g.mesh.position;
    if (!g.remote) {
      // Only the thrower records destruction; peers receive the rows.
      const keys = this.buildings.keysInRadius(point, WEAPON.grenadeRadius);
      if (keys.length > 0) this.events.emit("blocksDestroyed", { keys });
      // Splash damage with linear falloff — same authority model as
      // destruction: the thrower reports, the server clamps.
      const splash = this.playerTargets?.inRadius(point, WEAPON.grenadeRadius * 1.6) ?? [];
      for (const target of splash) {
        const falloff = 1 - target.dist / (WEAPON.grenadeRadius * 1.6);
        this.events.emit("playerHit", {
          avatarId: target.avatarId,
          point: point.clone(),
          damage: Math.max(10, Math.round(GRENADE_MAX_DAMAGE * falloff)),
        });
      }
    }
    this.events.emit("explosion", { point: point.clone(), radius: WEAPON.grenadeRadius });
    this.events.emit("shake", { strength: g.remote ? 0.18 : 0.35 });
  }

  update(ctx: FrameCtx) {
    this.timeNow = ctx.time;

    // Reload completion.
    if (this.reloading && ctx.time >= this.reloadEndsAt) {
      this.reloading = false;
      this.ammo = WEAPON.magSize;
    }
    if (this.ammo === 0 && !this.reloading) this.startReload();

    // Automatic fire (suppressed while dead).
    if (
      this.enabled &&
      this.firing &&
      !this.reloading &&
      this.ammo > 0 &&
      ctx.time - this.lastShotAt >= WEAPON.fireIntervalMs / 1000
    ) {
      this.fire(ctx);
    }

    // Grenade throw (rate-limited, suppressed while dead).
    if (this.queueGrenade) {
      this.queueGrenade = false;
      if (this.enabled && ctx.time - this.lastGrenadeAt >= WEAPON.grenadeCooldownMs / 1000) {
        this.throwGrenade(ctx);
      }
    }

    // Grenade physics.
    for (const g of this.grenades) {
      if (!g.active) continue;
      g.ttl -= ctx.dt;
      g.vel.y -= 16 * ctx.dt;
      g.mesh.position.addScaledVector(g.vel, ctx.dt);
      const p = g.mesh.position;
      const hitBuilding = this.buildings.raycastBlocks(
        this.tmpOrigin.copy(p).sub(this.tmpDir.copy(g.vel).normalize().multiplyScalar(0.2)),
        this.tmpDir,
        g.vel.length() * ctx.dt + 0.3,
      );
      const ground = Math.max(this.terrain.heightAt(p.x, p.z), WATER_LEVEL);
      if (g.ttl <= 0 || p.y <= ground + 0.1 || hitBuilding) {
        if (p.y < ground) p.y = ground + 0.2;
        this.detonate(g);
      }
    }

    // Viewmodel only exists in first person; the third-person rig
    // carries its own rifle.
    this.viewmodel.visible = this.player.viewMode === "first";

    // Viewmodel animation: recoil spring + reload dip + sway.
    this.viewKick *= Math.exp(-ctx.dt * 14);
    const reloadDip = this.reloading
      ? Math.sin(Math.min(1, (this.reloadEndsAt - ctx.time) / (WEAPON.reloadMs / 1000)) * Math.PI) * 0.22
      : 0;
    this.viewmodel.position.set(
      0.24,
      -0.2 - reloadDip,
      -0.5 + this.viewKick * 0.07,
    );
    this.viewmodel.rotation.x = this.viewKick * 0.12 - reloadDip * 0.8;

    // Muzzle flash decay.
    if (this.flashTtl > 0) {
      this.flashTtl -= ctx.dt;
      this.muzzleFlash.intensity = this.flashTtl > 0 ? 26 : 0;
    }
  }

  dispose() {
    window.removeEventListener("mousedown", this.onMouseDown);
    window.removeEventListener("mouseup", this.onMouseUp);
    window.removeEventListener("keydown", this.onKeyDown);
    window.removeEventListener("contextmenu", this.onContextMenu);
    this.camera.remove(this.viewmodel);
    this.viewmodel.traverse((o) => {
      const mesh = o as THREE.Mesh;
      if (mesh.geometry) mesh.geometry.dispose();
      if (mesh.material) (mesh.material as THREE.Material).dispose();
    });
    for (const g of this.grenades) {
      this.scene.remove(g.mesh);
    }
    // Pool shares one geometry/material pair.
    const first = this.grenades[0]?.mesh;
    if (first) {
      first.geometry.dispose();
      (first.material as THREE.Material).dispose();
    }
  }
}
