/**
 * First-person player controller.
 *
 * Pointer-lock mouse look, WASD + sprint + jump, gravity against the
 * terrain heightfield, and swimming: below sea level the controller
 * floats at the surface, moves slower, and bobs gently. Sprint widens
 * the FOV slightly; a subtle head-bob runs while grounded and moving.
 */
import * as THREE from "three";
import { GRAVITY, PLAYER, WATER_LEVEL, WORLD_HALF } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import type { Terrain } from "./terrain";

const BASE_FOV = 75;
const SPRINT_FOV = 84;

export type ViewMode = "first" | "third";

export class Player implements GameSystem {
  readonly name = "player";
  /** Eye position — the camera rig tracks this every frame. */
  readonly position = new THREE.Vector3(0, 10, 0);
  yaw = 0;
  pitch = 0;
  swimming = false;
  sprinting = false;
  /** Over-the-shoulder by default; V toggles first-person. */
  viewMode: ViewMode = "third";
  /** Death gate — movement input is ignored while down (look still works). */
  controlsEnabled = true;

  private readonly velocity = new THREE.Vector3();
  private readonly tmpForward = new THREE.Vector3();
  private readonly tmpRight = new THREE.Vector3();
  private readonly tmpBack = new THREE.Vector3();
  private grounded = false;
  private bobPhase = 0;
  private readonly keys = new Set<string>();
  private readonly domElement: HTMLElement;
  private recoilPitch = 0;

  // Bound handlers so dispose() can unregister them.
  private readonly onKeyDown = (e: KeyboardEvent) => {
    if (e.repeat) return;
    if (e.code === "KeyV" && document.pointerLockElement !== null) {
      this.viewMode = this.viewMode === "third" ? "first" : "third";
      return;
    }
    this.keys.add(e.code);
  };
  private readonly onKeyUp = (e: KeyboardEvent) => this.keys.delete(e.code);
  private readonly onMouseMove = (e: MouseEvent) => {
    if (document.pointerLockElement !== this.domElement) return;
    this.yaw -= e.movementX * 0.0022;
    this.pitch = Math.max(-1.5, Math.min(1.5, this.pitch - e.movementY * 0.0022));
  };
  private readonly onBlur = () => this.keys.clear();

  /** Extra boom occluder (buildings) — returns hit distance or null. */
  occluderTest:
    | ((origin: THREE.Vector3, dir: THREE.Vector3, far: number) => number | null)
    | null = null;

  constructor(
    private readonly terrain: Terrain,
    private readonly camera: THREE.PerspectiveCamera,
    domElement: HTMLElement,
    spawn: THREE.Vector3,
  ) {
    this.domElement = domElement;
    this.position.copy(spawn);
    window.addEventListener("keydown", this.onKeyDown);
    window.addEventListener("keyup", this.onKeyUp);
    document.addEventListener("mousemove", this.onMouseMove);
    window.addEventListener("blur", this.onBlur);
  }

  /** Weapon recoil feeds back into the view. Decays in update(). */
  kick(pitchUp: number) {
    this.recoilPitch += pitchUp;
  }

  /** Horizontal ground speed (m/s) — drives the body walk cycle. */
  get groundSpeed(): number {
    return Math.hypot(this.velocity.x, this.velocity.z);
  }

  /** Hard relocate (respawn) — zeroes momentum so you don't carry
   *  death-fall velocity into the new life. */
  teleport(to: THREE.Vector3) {
    this.position.copy(to);
    this.velocity.set(0, 0, 0);
  }

  isKeyDown(code: string): boolean {
    return this.keys.has(code);
  }

  update(ctx: FrameCtx) {
    const dt = ctx.dt;
    const ground = this.terrain.heightAt(this.position.x, this.position.z);
    const feetTarget = ground + PLAYER.eyeHeight;

    // Swimming when the terrain under us is deep enough that standing
    // would put our head underwater.
    const waterEye = WATER_LEVEL + 0.55;
    this.swimming = feetTarget < waterEye;

    // --- Movement input in view space (zeroed while dead) ---
    let fwd = 0;
    let strafe = 0;
    if (this.controlsEnabled) {
      if (this.keys.has("KeyW") || this.keys.has("ArrowUp")) fwd += 1;
      if (this.keys.has("KeyS") || this.keys.has("ArrowDown")) fwd -= 1;
      if (this.keys.has("KeyD") || this.keys.has("ArrowRight")) strafe += 1;
      if (this.keys.has("KeyA") || this.keys.has("ArrowLeft")) strafe -= 1;
    }
    const mag = Math.hypot(fwd, strafe);
    if (mag > 0) {
      fwd /= mag;
      strafe /= mag;
    }

    this.sprinting = this.keys.has("ShiftLeft") && fwd > 0 && !this.swimming;
    const speed = this.swimming
      ? PLAYER.swimSpeed
      : this.sprinting
        ? PLAYER.sprintSpeed
        : PLAYER.walkSpeed;

    const sin = Math.sin(this.yaw);
    const cos = Math.cos(this.yaw);
    // Forward is -z in view space at yaw 0.
    const moveX = (-sin * fwd + cos * strafe) * speed;
    const moveZ = (-cos * fwd - sin * strafe) * speed;

    if (this.swimming) {
      // --- Swim: float at the surface, damped vertical control ---
      this.velocity.x = moveX;
      this.velocity.z = moveZ;
      const surfaceY = waterEye + Math.sin(ctx.time * 1.6) * 0.08; // gentle bob
      this.velocity.y += (surfaceY - this.position.y) * 6 * dt - this.velocity.y * 3 * dt;
      if (this.keys.has("Space")) this.velocity.y += 4 * dt;
      this.grounded = false;
    } else {
      // --- Walk: instant horizontal control, gravity vertical ---
      this.velocity.x = moveX;
      this.velocity.z = moveZ;
      this.velocity.y -= GRAVITY * dt;
      if (this.grounded && this.keys.has("Space")) {
        this.velocity.y = PLAYER.jumpSpeed;
        this.grounded = false;
      }
    }

    this.position.addScaledVector(this.velocity, dt);

    // Terrain collision (eye height above ground).
    const groundNow = this.terrain.heightAt(this.position.x, this.position.z) + PLAYER.eyeHeight;
    if (!this.swimming && this.position.y <= groundNow) {
      this.position.y = groundNow;
      this.velocity.y = 0;
      this.grounded = true;
    } else if (!this.swimming) {
      this.grounded = false;
    }

    // World bounds.
    this.position.x = Math.max(-WORLD_HALF + 6, Math.min(WORLD_HALF - 6, this.position.x));
    this.position.z = Math.max(-WORLD_HALF + 6, Math.min(WORLD_HALF - 6, this.position.z));

    // --- Camera rig ---
    this.recoilPitch *= Math.exp(-dt * 9); // spring back

    let bobY = 0;
    const moving = mag > 0 && this.grounded;
    if (moving && this.viewMode === "first") {
      this.bobPhase += dt * (this.sprinting ? 13 : 9);
      bobY = Math.sin(this.bobPhase) * 0.035;
    } else {
      this.bobPhase = 0;
    }

    // Both modes share one orientation (yaw/pitch through the
    // crosshair); third person only OFFSETS the camera along a
    // shoulder boom, so the aim ray stays the camera ray.
    this.camera.rotation.set(this.pitch + this.recoilPitch, this.yaw, 0, "YXZ");
    if (this.viewMode === "first") {
      this.camera.position.set(this.position.x, this.position.y + bobY, this.position.z);
    } else {
      const fwd = this.tmpForward.set(
        -Math.sin(this.yaw) * Math.cos(this.pitch),
        Math.sin(this.pitch),
        -Math.cos(this.yaw) * Math.cos(this.pitch),
      );
      const right = this.tmpRight.set(Math.cos(this.yaw), 0, -Math.sin(this.yaw));
      // Tighter shoulder offset keeps the body close to the crosshair
      // line; the boom pulls in when terrain or buildings occlude it.
      const SHOULDER = 0.55;
      const boomLen = 3.4;
      let t = boomLen;
      for (let s = 1; s <= 6; s++) {
        const st = (boomLen * s) / 6;
        const sx = this.position.x - fwd.x * st + right.x * SHOULDER;
        const sy = this.position.y - fwd.y * st + 0.32;
        const sz = this.position.z - fwd.z * st + right.z * SHOULDER;
        if (sy < this.terrain.heightAt(sx, sz) + 0.35) {
          t = Math.max(0.6, st - boomLen / 6);
          break;
        }
      }
      if (this.occluderTest) {
        const back = this.tmpBack.copy(fwd).negate();
        const hit = this.occluderTest(this.position, back, boomLen);
        if (hit !== null) t = Math.min(t, Math.max(0.6, hit - 0.3));
      }
      this.camera.position.set(
        this.position.x - fwd.x * t + right.x * SHOULDER,
        Math.max(
          this.position.y - fwd.y * t + 0.32,
          this.terrain.heightAt(
            this.position.x - fwd.x * t + right.x * SHOULDER,
            this.position.z - fwd.z * t + right.z * SHOULDER,
          ) + 0.35,
        ),
        this.position.z - fwd.z * t + right.z * SHOULDER,
      );
    }

    const targetFov = this.sprinting ? SPRINT_FOV : BASE_FOV;
    if (Math.abs(this.camera.fov - targetFov) > 0.05) {
      this.camera.fov += (targetFov - this.camera.fov) * Math.min(1, dt * 8);
      this.camera.updateProjectionMatrix();
    }
  }

  dispose() {
    window.removeEventListener("keydown", this.onKeyDown);
    window.removeEventListener("keyup", this.onKeyUp);
    document.removeEventListener("mousemove", this.onMouseMove);
    window.removeEventListener("blur", this.onBlur);
  }
}
