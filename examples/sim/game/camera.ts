/**
 * RTS / city-builder camera: an angled view that orbits a ground focus
 * point. Pan with WASD or right-drag, zoom with the wheel, rotate with
 * Q/E. Left-click is reserved for painting tiles (handled elsewhere).
 */
import * as THREE from "three";
import { WORLD_HALF } from "./config";
import type { FrameCtx, GameSystem } from "./engine";

const MIN_DIST = 18;
const MAX_DIST = 150;
const POLAR = THREE.MathUtils.degToRad(52); // tilt from vertical

export class CameraRig implements GameSystem {
  readonly name = "camera";
  readonly target = new THREE.Vector3(0, 0, 0);

  private distance = 70;
  private azimuth = Math.PI * 0.25;
  private readonly keys = new Set<string>();
  private dragging = false;
  private lastX = 0;
  private lastY = 0;

  private readonly onKeyDown = (e: KeyboardEvent) => {
    if (["KeyW", "KeyA", "KeyS", "KeyD", "KeyQ", "KeyE", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(e.code))
      this.keys.add(e.code);
  };
  private readonly onKeyUp = (e: KeyboardEvent) => this.keys.delete(e.code);
  private readonly onBlur = () => this.keys.clear();

  private readonly onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const f = Math.exp(e.deltaY * 0.0012);
    this.distance = THREE.MathUtils.clamp(this.distance * f, MIN_DIST, MAX_DIST);
  };

  private readonly onPointerDown = (e: PointerEvent) => {
    // Right or middle button → pan-drag. Left is painting.
    if (e.button === 2 || e.button === 1) {
      this.dragging = true;
      this.lastX = e.clientX;
      this.lastY = e.clientY;
      this.dom.setPointerCapture(e.pointerId);
    }
  };
  private readonly onPointerMove = (e: PointerEvent) => {
    if (!this.dragging) return;
    const dx = e.clientX - this.lastX;
    const dy = e.clientY - this.lastY;
    this.lastX = e.clientX;
    this.lastY = e.clientY;
    this.panScreen(dx, dy);
  };
  private readonly onPointerUp = (e: PointerEvent) => {
    if (e.button === 2 || e.button === 1) this.dragging = false;
  };
  private readonly onContextMenu = (e: Event) => e.preventDefault();

  constructor(
    private readonly camera: THREE.PerspectiveCamera,
    private readonly dom: HTMLElement,
  ) {
    window.addEventListener("keydown", this.onKeyDown);
    window.addEventListener("keyup", this.onKeyUp);
    window.addEventListener("blur", this.onBlur);
    dom.addEventListener("wheel", this.onWheel, { passive: false });
    dom.addEventListener("pointerdown", this.onPointerDown);
    dom.addEventListener("pointermove", this.onPointerMove);
    dom.addEventListener("pointerup", this.onPointerUp);
    dom.addEventListener("contextmenu", this.onContextMenu);
    this.apply();
  }

  /** Screen-space ground axes derived from the current azimuth. */
  private axes(): { right: { x: number; z: number }; up: { x: number; z: number } } {
    const c = Math.cos(this.azimuth);
    const s = Math.sin(this.azimuth);
    // Matches apply(): camera sits at offset (sin az, cos az) from target,
    // so screen-right = (cos, -sin) and screen-up (forward) = (-sin, -cos).
    return { right: { x: c, z: -s }, up: { x: -s, z: -c } };
  }

  /** Grab-drag pan: the ground under the cursor follows it, so the
   *  focus point moves opposite to the screen-space drag. */
  private panScreen(dx: number, dy: number): void {
    const speed = this.distance * 0.0016;
    const { right, up } = this.axes();
    // drag vector on the ground = dx*right - dy*up (screen-down = -up).
    this.target.x -= (dx * right.x - dy * up.x) * speed;
    this.target.z -= (dx * right.z - dy * up.z) * speed;
    this.clampTarget();
  }

  private clampTarget(): void {
    const e = WORLD_HALF + 10;
    this.target.x = THREE.MathUtils.clamp(this.target.x, -e, e);
    this.target.z = THREE.MathUtils.clamp(this.target.z, -e, e);
  }

  private apply(): void {
    const sinP = Math.sin(POLAR);
    const r = this.distance;
    this.camera.position.set(
      this.target.x + r * sinP * Math.sin(this.azimuth),
      this.target.y + r * Math.cos(POLAR),
      this.target.z + r * sinP * Math.cos(this.azimuth),
    );
    this.camera.lookAt(this.target);
  }

  update(ctx: FrameCtx): void {
    const panV = this.distance * 0.9 * ctx.dt;
    let fwd = 0;
    let strafe = 0;
    if (this.keys.has("KeyW") || this.keys.has("ArrowUp")) fwd += 1;
    if (this.keys.has("KeyS") || this.keys.has("ArrowDown")) fwd -= 1;
    if (this.keys.has("KeyD") || this.keys.has("ArrowRight")) strafe += 1;
    if (this.keys.has("KeyA") || this.keys.has("ArrowLeft")) strafe -= 1;
    if (fwd || strafe) {
      const { right, up } = this.axes();
      // W pans the view up the screen, D pans it right.
      this.target.x += (strafe * right.x + fwd * up.x) * panV;
      this.target.z += (strafe * right.z + fwd * up.z) * panV;
      this.clampTarget();
    }
    if (this.keys.has("KeyQ")) this.azimuth += ctx.dt * 1.2;
    if (this.keys.has("KeyE")) this.azimuth -= ctx.dt * 1.2;
    this.apply();
  }

  dispose(): void {
    window.removeEventListener("keydown", this.onKeyDown);
    window.removeEventListener("keyup", this.onKeyUp);
    window.removeEventListener("blur", this.onBlur);
    this.dom.removeEventListener("wheel", this.onWheel);
    this.dom.removeEventListener("pointerdown", this.onPointerDown);
    this.dom.removeEventListener("pointermove", this.onPointerMove);
    this.dom.removeEventListener("pointerup", this.onPointerUp);
    this.dom.removeEventListener("contextmenu", this.onContextMenu);
  }
}
