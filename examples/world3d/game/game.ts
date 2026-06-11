/**
 * Composition root. Builds the three.js scene, registers every
 * GameSystem with the engine in update order, runs the render loop,
 * and exposes the narrow API the React page talks to:
 *
 *   setAvatars(rows)            ← Avatar live query
 *   syncDestruction(keys)       ← Destruction live query
 *   onStats(cb)                 → HUD numbers at 4 Hz
 *   requestPointerLock()        → click-to-play overlay
 */
import * as THREE from "three";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";
import { Buildings } from "./buildings";
import { QUALITY, SEED, WATER_LEVEL, WORLD_HALF, type QualityPreset } from "./config";
import { Debris } from "./debris";
import { Engine } from "./engine";
import { Net } from "./net";
import { Particles } from "./particles";
import { Player } from "./player";
import { makeRng } from "./prng";
import { RemotePlayers, type AvatarRow } from "./remote";
import { buildCharacter, type Character } from "./character";
import { Sky } from "./sky";
import { Terrain } from "./terrain";
import {
  makeBarkTexture,
  makeDetailTexture,
  makeFrondTexture,
  makeGrassClumpTexture,
  makeLeafTexture,
  makeNormalMapFromCanvas,
  makeRockTexture,
} from "./textures";
import { Vegetation } from "./vegetation";
import { Water } from "./water";
import { Weapon } from "./weapon";

/** Procedural texture bundle shared by the world systems. */
export interface WorldTextures {
  detail: THREE.Texture;
  detailFine: THREE.Texture;
  frond: THREE.Texture;
  leaf: THREE.Texture;
  bark: THREE.Texture;
  rock: THREE.Texture;
  rockNormal: THREE.Texture;
  concrete: THREE.Texture;
  concreteNormal: THREE.Texture;
  grassClump: THREE.Texture;
}

export interface GameStats {
  fps: number;
  drawCalls: number;
  triangles: number;
  players: number;
  ammo: number;
  reloading: boolean;
  swimming: boolean;
  blocksDestroyed: number;
  blocksTotal: number;
  p50: number;
  p95: number;
  mutPerSec: number;
  /** Player pose + remote dots for the minimap (world coords). */
  pose: { x: number; z: number; heading: number };
  remotes: Array<{ x: number; z: number; color: string }>;
}

/** Wire shape for fire-event broadcasts (kept terse — ~10 Hz/player). */
export interface FireEvent {
  k: "s" | "g"; // shot | grenade
  o: [number, number, number]; // origin
  d: [number, number, number]; // direction (shot) or velocity (grenade)
}

export class Game {
  readonly spawnPoint: THREE.Vector3;

  private readonly renderer: THREE.WebGLRenderer;
  private readonly composer: EffectComposer;
  private readonly scene = new THREE.Scene();
  private readonly camera: THREE.PerspectiveCamera;
  private readonly engine = new Engine();
  private readonly textures: WorldTextures;

  private readonly terrain: Terrain;
  private readonly buildings: Buildings;
  private readonly debris: Debris;
  private readonly particles: Particles;
  private readonly player: Player;
  private readonly weapon: Weapon;
  private readonly remote: RemotePlayers;
  private readonly net: Net;
  private readonly sky: Sky;

  private readonly selfRig: Character;
  private selfColorSet = false;
  private readonly aimTarget = new THREE.Vector3();
  private readonly camDir = new THREE.Vector3();
  private readonly muzzleWorld = new THREE.Vector3();
  private raf = 0;
  private lastFrameAt = performance.now();
  private running = false;
  private statsCb: ((stats: GameStats) => void) | null = null;
  private fireCb: ((ev: FireEvent) => void) | null = null;
  private statsTimer = 0;
  private fpsFrames = 0;
  private fpsWindowStart = performance.now();
  private fps = 0;
  private shakeStrength = 0;
  private readonly disposeFns: Array<() => void> = [];

  constructor(
    private readonly container: HTMLElement,
    quality: QualityPreset = QUALITY.high,
  ) {
    // --- Renderer ---
    this.renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: "high-performance" });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, quality.pixelRatioCap));
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
    this.renderer.toneMappingExposure = 1.12;
    // EffectComposer runs multiple passes per frame; with autoReset on,
    // renderer.info only ever reflects the LAST pass (1 draw call).
    this.renderer.info.autoReset = false;
    container.appendChild(this.renderer.domElement);

    this.camera = new THREE.PerspectiveCamera(75, 1, 0.1, 1200);
    this.scene.add(this.camera); // viewmodel is parented to the camera

    // Distance haze — also the far-clip budget that keeps draw work
    // bounded. The same color feeds the sky's horizon band and the
    // grass/water shaders so everything dissolves into one atmosphere.
    const fog = new THREE.FogExp2("#ccd6d9", 0.0052);
    this.scene.fog = fog;

    // --- Procedural textures (canvas-generated, no downloads) ---
    const rock = makeRockTexture(2);
    const concrete = makeRockTexture(1);
    this.textures = {
      detail: makeDetailTexture(96, 0.2),
      detailFine: makeDetailTexture(3, 0.3),
      frond: makeFrondTexture(),
      leaf: makeLeafTexture(),
      bark: makeBarkTexture(),
      rock,
      rockNormal: makeNormalMapFromCanvas(rock.image as HTMLCanvasElement, 1.6, 2),
      concrete,
      concreteNormal: makeNormalMapFromCanvas(concrete.image as HTMLCanvasElement, 1.2, 1),
      grassClump: makeGrassClumpTexture(),
    };

    // --- World (deterministic from SEED) ---
    this.terrain = new Terrain(this.textures.detail);

    // Compound sites get terraformed pads BEFORE anything reads the
    // heightfield: buildings sit flush, vegetation keeps clear, and
    // the water/minimap see the final terrain.
    const sites = Buildings.planSites(this.terrain, 7);
    for (const site of sites) {
      site.y = this.terrain.flattenArea(site.x, site.z, 11, 12);
    }
    this.terrain.refreshGeometry();
    this.scene.add(this.terrain.mesh);
    const clearings = sites.map((s) => ({ x: s.x, z: s.z, r: 13 }));

    this.sky = this.engine.add(new Sky(this.scene, fog.color, quality));

    const water = this.engine.add(new Water(this.terrain, this.sky.sunDir, fog));
    this.scene.add(water.mesh);

    const vegetation = this.engine.add(
      new Vegetation(this.terrain, fog, this.sky.sunDir, this.textures, clearings, quality),
    );
    this.scene.add(vegetation.group);

    this.buildings = this.engine.add(
      new Buildings(this.terrain, sites, this.textures.concrete, this.textures.concreteNormal),
    );
    this.scene.add(this.buildings.mesh);

    // --- Gameplay systems ---
    this.spawnPoint = this.pickSpawn(vegetation);
    this.player = this.engine.add(
      new Player(this.terrain, this.camera, this.renderer.domElement, this.spawnPoint),
    );
    this.weapon = this.engine.add(
      new Weapon(this.scene, this.camera, this.player, this.terrain, this.buildings, this.engine.events),
    );
    // Third-person boom respects walls.
    this.player.occluderTest = (origin, dir, far) =>
      this.buildings.raycastDistance(origin, dir, far);

    this.remote = this.engine.add(new RemotePlayers(this.terrain));
    this.scene.add(this.remote.group);

    this.debris = this.engine.add(new Debris(this.terrain, this.textures.concrete));
    this.scene.add(this.debris.mesh);

    this.particles = this.engine.add(new Particles(this.engine.events));
    this.scene.add(this.particles.points, this.particles.tracerGroup);

    this.net = this.engine.add(new Net(this.player, this.engine.events));

    // --- Local third-person body (visible when the camera is back) ---
    this.selfRig = buildCharacter("#cdd3da");
    this.scene.add(this.selfRig.group);

    // --- Cross-system effects ---
    const offShot = this.engine.events.on("shot", ({ origin, direction }) => {
      const dist =
        this.buildings.raycastDistance(origin, direction, 220) ?? 120;
      // Tracer starts at the character's actual muzzle in third person
      // (the camera sits 3.6 m behind the body), or just under the
      // camera in first person.
      const muzzle =
        this.player.viewMode === "first"
          ? origin.clone().addScaledVector(direction, 1.2).add(new THREE.Vector3(0, -0.12, 0))
          : this.selfRig.muzzle.getWorldPosition(this.muzzleWorld).clone();
      this.particles.tracer(muzzle, origin.clone().addScaledVector(direction, dist));
      this.selfRig.fire();
      this.fireCb?.({
        k: "s",
        o: [origin.x, origin.y, origin.z],
        d: [direction.x, direction.y, direction.z],
      });
    });
    const offGrenade = this.engine.events.on("grenadeThrown", ({ origin, velocity }) => {
      this.fireCb?.({
        k: "g",
        o: [origin.x, origin.y, origin.z],
        d: [velocity.x, velocity.y, velocity.z],
      });
    });
    const offDestroyed = this.engine.events.on("blocksDestroyed", ({ keys }) => {
      this.spawnDestructionFx(this.buildings.destroyKeys(keys));
    });
    const offShake = this.engine.events.on("shake", ({ strength }) => {
      this.shakeStrength = Math.min(0.6, this.shakeStrength + strength);
    });
    this.disposeFns.push(offShot, offGrenade, offDestroyed, offShake);

    // --- Post-processing: render → bloom → output ---
    // Bloom picks up the sun disc, glints, muzzle flash, and explosion
    // particles — the soft HDR glow that flat rendering lacks. The
    // threshold keeps midtone foliage out of it.
    this.composer = new EffectComposer(this.renderer);
    this.composer.addPass(new RenderPass(this.scene, this.camera));
    const bloom = new UnrealBloomPass(new THREE.Vector2(1, 1), 0.35, 0.6, 0.85);
    this.composer.addPass(bloom);
    this.composer.addPass(new OutputPass());

    // --- Resize / context loss ---
    const resize = () => {
      const rect = container.getBoundingClientRect();
      this.renderer.setSize(rect.width, rect.height, false);
      this.composer.setSize(rect.width, rect.height);
      this.camera.aspect = rect.width / Math.max(1, rect.height);
      this.camera.updateProjectionMatrix();
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(container);
    this.disposeFns.push(() => ro.disconnect());

    const onContextLost = (e: Event) => {
      e.preventDefault();
      console.warn("[world3d] WebGL context lost — reload to recover");
    };
    this.renderer.domElement.addEventListener("webglcontextlost", onContextLost);
    this.disposeFns.push(() =>
      this.renderer.domElement.removeEventListener("webglcontextlost", onContextLost),
    );
  }

  /**
   * Top-down island image for the minimap, rendered once from the
   * heightfield: depth-shaded water, sand line, greens by altitude,
   * rock on the peaks, plus building footprints.
   */
  buildMinimapImage(): HTMLCanvasElement {
    const size = 256;
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = size;
    const ctx = canvas.getContext("2d")!;
    const img = ctx.createImageData(size, size);
    const half = WORLD_HALF;

    for (let py = 0; py < size; py++) {
      for (let px = 0; px < size; px++) {
        const wx = (px / (size - 1)) * 2 * half - half;
        const wz = (py / (size - 1)) * 2 * half - half;
        const h = this.terrain.heightAt(wx, wz);
        const slope = this.terrain.slopeAt(wx, wz);

        let r: number, g: number, b: number;
        if (h < WATER_LEVEL) {
          const depth = Math.min(1, (WATER_LEVEL - h) / 12);
          r = 30 + (1 - depth) * 60;
          g = 90 + (1 - depth) * 90;
          b = 130 + (1 - depth) * 60;
        } else if (h < WATER_LEVEL + 1.6) {
          r = 205; g = 185; b = 140; // sand
        } else if (h > 26 || slope > 0.34) {
          r = 125; g = 125; b = 122; // rock
        } else {
          const t = Math.min(1, (h - 1.6) / 28);
          r = 72 - t * 25; g = 110 - t * 35; b = 42 - t * 12; // greens
        }
        // Hillshade from the sun direction.
        const normal = this.terrain.normalAt(wx, wz);
        const shade = 0.65 + 0.35 * Math.max(0, normal.dot(this.sky.sunDir));
        const i = (py * size + px) * 4;
        img.data[i] = Math.round(r * shade);
        img.data[i + 1] = Math.round(g * shade);
        img.data[i + 2] = Math.round(b * shade);
        img.data[i + 3] = 255;
      }
    }
    ctx.putImageData(img, 0, 0);

    // Building footprints.
    ctx.fillStyle = "#d8d4c8";
    for (const site of this.buildings.siteFootprints()) {
      const px = ((site.x + half) / (2 * half)) * size;
      const py = ((site.z + half) / (2 * half)) * size;
      const s = Math.max(2.5, (site.size / (2 * half)) * size);
      ctx.fillRect(px - s / 2, py - s / 2, s, s);
    }
    return canvas;
  }

  /** A dry, flattish, TREE-FREE spot — the third-person boom needs
   *  ~4 m of clear air behind the player at spawn. */
  private pickSpawn(vegetation: Vegetation): THREE.Vector3 {
    const rng = makeRng(SEED ^ 0x5fa3);
    for (let i = 0; i < 400; i++) {
      const x = rng.range(-120, 120);
      const z = rng.range(-120, 120);
      const h = this.terrain.heightAt(x, z);
      if (
        h > WATER_LEVEL + 2 &&
        h < WATER_LEVEL + 12 &&
        this.terrain.slopeAt(x, z) < 0.12 &&
        vegetation.clearOfTrees(x, z, 6)
      ) {
        return new THREE.Vector3(x, h + 1.7, z);
      }
    }
    return new THREE.Vector3(0, this.terrain.heightAt(0, 0) + 1.7, 0);
  }

  private spawnDestructionFx(removed: Array<{ key: string; center: THREE.Vector3 }>) {
    // Cap per-event effect volume so a building collapse can't hitch the frame.
    const maxFx = 48;
    const step = Math.max(1, Math.ceil(removed.length / maxFx));
    for (let i = 0; i < removed.length; i += step) {
      const r = removed[i];
      const color = this.buildings.colorOf(r.key);
      const impulse = r.center.clone().sub(this.player.position).normalize().multiplyScalar(3);
      this.debris.spawnFromBlock(r.center, color, impulse, 2);
      this.particles.spawn({
        position: r.center,
        count: 3,
        color: "#b6ad9d",
        speed: [1, 4],
        up: 2,
        size: [4, 9],
        life: [0.4, 0.9],
        gravity: 5,
      });
    }
  }

  // -----------------------------------------------------------------------
  // React-facing API
  // -----------------------------------------------------------------------

  setAvatars(rows: AvatarRow[], selfUserId: string | null) {
    this.remote.setAvatars(rows, selfUserId);
    // Tint the local body with our server-assigned color once known.
    if (!this.selfColorSet && selfUserId) {
      const mine = rows.find((r) => r.userId === selfUserId);
      if (mine) {
        this.selfRig.bodyMat.color.set(mine.color);
        this.selfColorSet = true;
      }
    }
  }

  /** Register the outbound fire-event broadcaster (rooms WS relay). */
  onLocalFire(cb: (ev: FireEvent) => void) {
    this.fireCb = cb;
  }

  /** Dev-console: drop a character 3 m ahead, facing the camera. */
  debugSpawnDummy(): void {
    const dummy = buildCharacter("#e0b13d");
    const dir = new THREE.Vector3();
    this.camera.getWorldDirection(dir);
    dummy.group.position
      .setFromMatrixPosition(this.camera.matrixWorld)
      .addScaledVector(dir, 3.2);
    dummy.group.position.y =
      this.terrain.heightAt(dummy.group.position.x, dummy.group.position.z) + 1.62;
    dummy.group.rotation.y = Math.atan2(dir.x, dir.z); // face the camera
    this.scene.add(dummy.group);
    const tick = () => {
      dummy.animate(performance.now() / 1000, 0);
      if (this.running) requestAnimationFrame(tick);
    };
    tick();
  }

  /** Dev-console introspection (see window.__world3d). */
  debugRig(): Record<string, unknown> {
    const rifleWorldPos = new THREE.Vector3();
    const rifleWorldScale = new THREE.Vector3();
    const rifle = this.selfRig.muzzle.parent;
    rifle?.getWorldPosition(rifleWorldPos);
    rifle?.getWorldScale(rifleWorldScale);
    return {
      player: this.player.position.toArray().map((n) => +n.toFixed(2)),
      rifleParent: rifle?.parent?.name || rifle?.parent?.type,
      rifleChildren: rifle?.children.map((c) => c.name || c.type),
      rifleWorldPos: rifleWorldPos.toArray().map((n) => +n.toFixed(2)),
      rifleWorldScale: +rifleWorldScale.x.toFixed(4),
    };
  }

  /**
   * Play a peer's fire event. Purely cosmetic — any destruction it
   * caused arrives separately as Destruction rows from the shooter.
   */
  applyRemoteFire(ev: FireEvent, fromUserId?: string) {
    if (fromUserId) this.remote.fireCharacter(fromUserId);
    const origin = new THREE.Vector3(ev.o[0], ev.o[1], ev.o[2]);
    const d = new THREE.Vector3(ev.d[0], ev.d[1], ev.d[2]);
    if (ev.k === "g") {
      this.weapon.spawnRemoteGrenade(origin, d);
      return;
    }
    const direction = d.normalize();
    const dist = this.buildings.raycastDistance(origin, direction, 220) ?? 120;
    const muzzle = origin.clone().addScaledVector(direction, 0.9);
    const hit = origin.clone().addScaledVector(direction, dist);
    this.particles.tracer(muzzle, hit);
    this.particles.spawn({
      position: hit,
      count: 4,
      color: "#b9b2a4",
      speed: [1, 3],
      up: 1.5,
      size: [3, 6],
      life: [0.2, 0.45],
      gravity: 6,
    });
  }

  setAvatarId(id: string) {
    this.net.setAvatarId(id);
  }

  /** Reconcile building state with the Destruction live query. */
  syncDestruction(keys: ReadonlySet<string>) {
    const removed = this.buildings.syncDestroyedKeys(keys);
    // Remote demolition still kicks up dust on our screen.
    if (removed.length > 0) this.spawnDestructionFx(removed);
  }

  onStats(cb: (stats: GameStats) => void) {
    this.statsCb = cb;
  }

  requestPointerLock() {
    this.renderer.domElement.requestPointerLock();
  }

  start() {
    if (this.running) return;
    this.running = true;
    this.lastFrameAt = performance.now();
    const loop = (now: number) => {
      if (!this.running) return;
      this.raf = requestAnimationFrame(loop);
      const dt = (now - this.lastFrameAt) / 1000;
      this.lastFrameAt = now;

      const ctx = this.engine.tick(dt, this.camera, this.player.position);

      // Third-person body follows the controller (origin at eye level,
      // same convention as remote characters). Hidden in first person.
      this.selfRig.group.visible = this.player.viewMode === "third";
      if (this.selfRig.group.visible) {
        this.selfRig.group.position.copy(this.player.position);
        this.selfRig.group.rotation.y = this.player.yaw;
        this.selfRig.animate(ctx.time, this.player.groundSpeed);
        // Converge the rifle on the CROSSHAIR target, not a parallel
        // line — kills the shoulder-cam parallax where the gun looked
        // offset from where shots land. Computed in the rig's local
        // frame (lookAt points +Z at targets; the barrel runs -Z).
        this.camera.getWorldDirection(this.camDir);
        const aimDist =
          this.buildings.raycastDistance(
            this.camera.getWorldPosition(this.aimTarget),
            this.camDir,
            120,
          ) ?? 40;
        this.aimTarget
          .setFromMatrixPosition(this.camera.matrixWorld)
          .addScaledVector(this.camDir, aimDist);
        const p = this.player.position;
        const dx = this.aimTarget.x - p.x;
        const dy = this.aimTarget.y - (p.y - 0.3); // pivot sits 0.3 below eye
        const dz = this.aimTarget.z - p.z;
        const cy = Math.cos(this.player.yaw);
        const sy = Math.sin(this.player.yaw);
        const lx = dx * cy - dz * sy; // world → body-local (inverse yaw)
        const lz = dx * sy + dz * cy;
        this.selfRig.aimPivot.rotation.y = Math.atan2(-lx, -lz);
        this.selfRig.aimPivot.rotation.x = Math.atan2(dy, Math.hypot(lx, lz));
      }

      // Camera shake (post-systems so it layers over player look).
      if (this.shakeStrength > 0.001) {
        this.shakeStrength *= Math.exp(-ctx.dt * 6);
        this.camera.rotation.x += (Math.random() - 0.5) * this.shakeStrength * 0.06;
        this.camera.rotation.z += (Math.random() - 0.5) * this.shakeStrength * 0.04;
      }

      this.renderer.info.reset();
      this.composer.render();

      // FPS over a rolling 0.5 s window.
      this.fpsFrames++;
      if (now - this.fpsWindowStart >= 500) {
        this.fps = Math.round((this.fpsFrames * 1000) / (now - this.fpsWindowStart));
        this.fpsFrames = 0;
        this.fpsWindowStart = now;
      }

      // HUD callback at 4 Hz — keeps React re-renders off the hot path.
      this.statsTimer += ctx.dt;
      if (this.statsTimer >= 0.25 && this.statsCb) {
        this.statsTimer = 0;
        const info = this.renderer.info;
        const blocks = this.buildings.stats;
        const netStats = this.net.hudStats;
        this.statsCb({
          fps: this.fps,
          drawCalls: info.render.calls,
          triangles: info.render.triangles,
          players: this.remote.count + 1,
          ammo: this.weapon.ammo,
          reloading: this.weapon.reloading,
          swimming: this.player.swimming,
          blocksDestroyed: blocks.destroyed,
          blocksTotal: blocks.total,
          pose: {
            x: this.player.position.x,
            z: this.player.position.z,
            heading: this.player.yaw,
          },
          remotes: this.remote.minimapDots(),
          ...netStats,
        });
      }
    };
    this.raf = requestAnimationFrame(loop);
  }

  dispose() {
    this.running = false;
    cancelAnimationFrame(this.raf);
    for (const fn of this.disposeFns) fn();
    this.engine.dispose(); // disposes all systems in reverse order
    this.selfRig.dispose();
    this.terrain.dispose();
    for (const tex of Object.values(this.textures)) tex.dispose();
    this.composer.dispose();
    this.renderer.dispose();
    if (this.container.contains(this.renderer.domElement)) {
      this.container.removeChild(this.renderer.domElement);
    }
  }
}
