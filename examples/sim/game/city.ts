/**
 * Composition root for the city builder. Owns the renderer, scene,
 * lights, the engine + its systems (camera, tiles, net), the render
 * loop, and mouse painting. React (page.tsx) talks to it through a
 * narrow setter API: setTool / setTiles / setCity / onStats.
 */
import * as THREE from "three";
import { CameraRig } from "./camera";
import { GRID, TILE, WORLD_HALF } from "./config";
import { Engine, type TileKind } from "./engine";
import { cellCenterX, cellCenterZ, inBounds, makeBorder, makeCursor, makeGround, worldToCell } from "./grid";
import { preloadKit } from "./kit";
import { Net } from "./net";
import { TileMap } from "./tiles";

export type Tool = "road" | "res" | "com" | "ind" | "bulldoze" | "pan";

export interface CityRow {
  funds: number;
  population: number;
  jobs: number;
  happiness: number;
  tick: number;
}

export interface CityStats {
  fps: number;
  draws: number;
  tiles: number;
  funds: number;
  population: number;
  jobs: number;
  happiness: number;
  tick: number;
  tool: Tool;
  mutPerSec: number;
}

export class City {
  readonly spawn = new THREE.Vector3(0, 0, 0);

  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera: THREE.PerspectiveCamera;
  private readonly engine = new Engine();
  private readonly rig: CameraRig;
  private readonly tiles: TileMap;
  private readonly net: Net;
  private readonly sun: THREE.DirectionalLight;
  private readonly cursor: THREE.Mesh;

  private readonly raycaster = new THREE.Raycaster();
  private readonly groundPlane = new THREE.Plane(new THREE.Vector3(0, 1, 0), 0);
  private readonly ndc = new THREE.Vector2();
  private readonly hit = new THREE.Vector3();

  private tool: Tool = "road";
  private hovered: { gx: number; gz: number } | null = null;
  private painting = false;
  private stroke = new Set<string>();

  private city: CityRow = { funds: 0, population: 0, jobs: 0, happiness: 100, tick: 0 };
  private statsCb: ((s: CityStats) => void) | null = null;
  private running = false;
  private raf = 0;
  private lastFrameAt = performance.now();
  private fps = 0;
  private fpsAccum = 0;
  private fpsFrames = 0;
  private statsAccum = 0;

  constructor(private readonly container: HTMLElement) {
    const rect = container.getBoundingClientRect();
    this.renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: "high-performance" });
    this.renderer.setPixelRatio(Math.min(2, window.devicePixelRatio));
    this.renderer.setSize(rect.width, rect.height, false);
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    this.renderer.info.autoReset = false;
    container.appendChild(this.renderer.domElement);

    this.scene.background = new THREE.Color(0x9ec7e0);
    this.scene.fog = new THREE.Fog(0x9ec7e0, WORLD_HALF * 1.4, WORLD_HALF * 3);

    this.camera = new THREE.PerspectiveCamera(50, rect.width / Math.max(1, rect.height), 0.5, 2000);

    // Lights: soft sky fill + a low sun for long stylised shadows.
    const hemi = new THREE.HemisphereLight(0xcfe4f0, 0x3a4a40, 1.0);
    this.scene.add(hemi);
    this.sun = new THREE.DirectionalLight(0xfff2d8, 1.5);
    this.sun.castShadow = true;
    this.sun.shadow.mapSize.set(2048, 2048);
    this.sun.shadow.camera.near = 1;
    this.sun.shadow.camera.far = 260;
    const s = 70; // shadow frustum half-size, follows the camera target
    this.sun.shadow.camera.left = -s;
    this.sun.shadow.camera.right = s;
    this.sun.shadow.camera.top = s;
    this.sun.shadow.camera.bottom = -s;
    this.sun.shadow.bias = -0.0004;
    this.scene.add(this.sun);
    this.scene.add(this.sun.target);

    this.scene.add(makeGround());
    this.scene.add(makeBorder());
    this.cursor = makeCursor();
    this.scene.add(this.cursor);

    // Engine systems (order = update order).
    this.rig = this.engine.add(new CameraRig(this.camera, this.renderer.domElement));
    this.tiles = this.engine.add(new TileMap(this.engine.events));
    this.net = this.engine.add(new Net(this.engine.events));
    this.scene.add(this.tiles.group);

    // Async-load the Quaternius kit; procedural fallback renders until
    // it arrives, then rebuild buildings with the real models.
    preloadKit()
      .then(() => this.tiles.refreshKit())
      .catch(() => {});

    // Painting interaction (left button).
    const dom = this.renderer.domElement;
    dom.addEventListener("pointerdown", this.onPointerDown);
    dom.addEventListener("pointermove", this.onPointerMove);
    window.addEventListener("pointerup", this.onPointerUp);

    const resize = () => {
      const r = container.getBoundingClientRect();
      this.renderer.setSize(r.width, r.height, false);
      this.camera.aspect = r.width / Math.max(1, r.height);
      this.camera.updateProjectionMatrix();
    };
    const ro = new ResizeObserver(resize);
    ro.observe(container);
    this.disposeFns.push(() => ro.disconnect());
  }

  private disposeFns: Array<() => void> = [];

  // --- React-facing API ---

  setTool(tool: Tool): void {
    this.tool = tool;
  }

  setTiles(rows: Array<{ gx: number; gz: number; kind: string; level: number }>): void {
    this.tiles.setTiles(rows);
  }

  setCity(row: CityRow | null): void {
    if (row) this.city = row;
  }

  onStats(cb: (s: CityStats) => void): void {
    this.statsCb = cb;
  }

  // --- Interaction ---

  private updateHover(clientX: number, clientY: number): void {
    const rect = this.renderer.domElement.getBoundingClientRect();
    this.ndc.x = ((clientX - rect.left) / rect.width) * 2 - 1;
    this.ndc.y = -((clientY - rect.top) / rect.height) * 2 + 1;
    this.raycaster.setFromCamera(this.ndc, this.camera);
    if (!this.raycaster.ray.intersectPlane(this.groundPlane, this.hit)) {
      this.hovered = null;
      this.cursor.visible = false;
      return;
    }
    const { gx, gz } = worldToCell(this.hit.x, this.hit.z);
    if (!inBounds(gx, gz)) {
      this.hovered = null;
      this.cursor.visible = false;
      return;
    }
    this.hovered = { gx, gz };
    this.cursor.position.set(cellCenterX(gx), 0.04, cellCenterZ(gz));
    this.cursor.visible = true;
    const occupied = this.tiles.kindAt(gx, gz) !== null;
    const valid = this.tool === "bulldoze" ? occupied : this.tool === "pan" ? false : !occupied;
    (this.cursor.material as THREE.MeshBasicMaterial).color.setHex(
      this.tool === "bulldoze" ? 0xff6b5a : valid ? 0x8effa0 : 0xff6b5a,
    );
    this.cursor.visible = this.tool !== "pan";
  }

  private paintHovered(): void {
    if (!this.hovered || this.tool === "pan") return;
    const { gx, gz } = this.hovered;
    const key = gx + "," + gz;
    if (this.stroke.has(key)) return;
    this.stroke.add(key);
    const occupied = this.tiles.kindAt(gx, gz) !== null;
    if (this.tool === "bulldoze") {
      if (occupied) this.engine.events.emit("tilesBulldozed", { cells: [{ gx, gz }] });
    } else if (!occupied) {
      this.engine.events.emit("tilesPainted", { kind: this.tool as TileKind, cells: [{ gx, gz }] });
    }
  }

  private readonly onPointerDown = (e: PointerEvent) => {
    if (e.button !== 0 || this.tool === "pan") return;
    this.painting = true;
    this.stroke.clear();
    this.updateHover(e.clientX, e.clientY);
    this.paintHovered();
  };
  private readonly onPointerMove = (e: PointerEvent) => {
    this.updateHover(e.clientX, e.clientY);
    if (this.painting) this.paintHovered();
  };
  private readonly onPointerUp = () => {
    this.painting = false;
    this.stroke.clear();
  };

  // --- Loop ---

  start(): void {
    if (this.running) return;
    this.running = true;
    this.lastFrameAt = performance.now();
    const loop = (now: number) => {
      if (!this.running) return;
      this.raf = requestAnimationFrame(loop);
      const dt = (now - this.lastFrameAt) / 1000;
      this.lastFrameAt = now;

      this.engine.tick(dt, this.camera);

      // Keep the sun's shadow frustum over the camera focus for crisp
      // local shadows on a large map.
      const t = this.rig.target;
      this.sun.position.set(t.x + 60, 120, t.z + 40);
      this.sun.target.position.set(t.x, 0, t.z);

      this.renderer.info.reset();
      this.renderer.render(this.scene, this.camera);

      // FPS (smoothed) + stats at 4 Hz.
      this.fpsAccum += dt;
      this.fpsFrames++;
      if (this.fpsAccum >= 0.5) {
        this.fps = Math.round(this.fpsFrames / this.fpsAccum);
        this.fpsAccum = 0;
        this.fpsFrames = 0;
      }
      this.statsAccum += dt;
      if (this.statsAccum >= 0.25 && this.statsCb) {
        this.statsAccum = 0;
        this.statsCb({
          fps: this.fps,
          draws: this.renderer.info.render.calls,
          tiles: this.tiles.tileCount,
          funds: this.city.funds,
          population: this.city.population,
          jobs: this.city.jobs,
          happiness: this.city.happiness,
          tick: this.city.tick,
          tool: this.tool,
          mutPerSec: this.net.mutPerSec,
        });
      }
    };
    this.raf = requestAnimationFrame(loop);
  }

  dispose(): void {
    this.running = false;
    cancelAnimationFrame(this.raf);
    const dom = this.renderer.domElement;
    dom.removeEventListener("pointerdown", this.onPointerDown);
    dom.removeEventListener("pointermove", this.onPointerMove);
    window.removeEventListener("pointerup", this.onPointerUp);
    for (const fn of this.disposeFns) fn();
    this.engine.dispose();
    this.renderer.dispose();
    if (dom.parentElement === this.container) this.container.removeChild(dom);
  }
}
