/**
 * Composition root for the city builder. Owns the renderer, scene,
 * lights, the engine + its systems (camera, tiles, net), the render
 * loop, and mouse painting. React (page.tsx) talks to it through a
 * narrow setter API: setTool / setTiles / setCity / onStats.
 */
import * as THREE from "three";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";
import { CameraRig } from "./camera";
import { GRID, TILE, WORLD_HALF } from "./config";
import { Engine, type TileKind } from "./engine";
import { cellCenterX, cellCenterZ, inBounds, makeCursor, worldToCell } from "./grid";
import { preloadKit } from "./kit";
import { Net } from "./net";
import { Sky } from "./sky";
import { buildTerrainMesh, buildWater, heightAt, isBuildableCell } from "./terrain";
import { TileMap } from "./tiles";
import { preloadTrees, Vegetation } from "./vegetation";

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
  private readonly vegetation: Vegetation;
  private readonly sky: Sky;
  private readonly terrain: THREE.Mesh;
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
    // ACES tone mapping + sRGB so the PBR brick/asphalt read with proper
    // contrast instead of muddy browns.
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
    this.renderer.toneMappingExposure = 0.92;
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    container.appendChild(this.renderer.domElement);

    this.scene.background = new THREE.Color(0xaed2ea);

    // Image-based lighting: a neutral room env so MeshStandardMaterial
    // (the GLB brick/glass/asphalt) has reflections and doesn't render
    // dark/flat. The Sky scales environmentIntensity over the day.
    const pmrem = new THREE.PMREMGenerator(this.renderer);
    this.scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;

    this.camera = new THREE.PerspectiveCamera(50, rect.width / Math.max(1, rect.height), 0.5, 4000);

    // Sky owns all lighting: sun + moon + hemisphere, a gradient sky
    // dome, fog, and the day/night cycle. Its sun casts the shadows.
    this.sky = new Sky(this.scene);

    // 3-D terrain (heightfield with mountains/valleys/river/lake) + water.
    this.terrain = buildTerrainMesh();
    this.scene.add(this.terrain);
    this.scene.add(buildWater());

    this.cursor = makeCursor();
    this.scene.add(this.cursor);

    // Engine systems (order = update order).
    this.rig = this.engine.add(new CameraRig(this.camera, this.renderer.domElement));
    this.tiles = this.engine.add(new TileMap(this.engine.events));
    this.net = this.engine.add(new Net(this.engine.events));
    this.vegetation = this.engine.add(new Vegetation());
    this.scene.add(this.tiles.group);
    this.scene.add(this.vegetation.group);

    // Async-load the Quaternius kit; procedural fallback renders until
    // it arrives, then rebuild buildings with the real models.
    preloadKit()
      .then(() => this.tiles.refreshKit())
      .catch(() => {});
    // Trees scatter once loaded; rescatter when the built area changes.
    preloadTrees()
      .then(() => this.rescatterTrees())
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
    this.rescatterTrees();
  }

  /** Keep the forest clear of the built area (no-op if unchanged). */
  private rescatterTrees(): void {
    this.vegetation.scatter(
      this.tiles.occupiedCells(),
      (gx, gz) => this.tiles.isRoadAt(gx, gz),
    );
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
    // Hit the real terrain surface (falls back to the sea-level plane
    // where the ray misses land, e.g. over water).
    const hits = this.raycaster.intersectObject(this.terrain, false);
    if (hits.length > 0) {
      this.hit.copy(hits[0].point);
    } else if (!this.raycaster.ray.intersectPlane(this.groundPlane, this.hit)) {
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
    const cx = cellCenterX(gx);
    const cz = cellCenterZ(gz);
    this.cursor.position.set(cx, heightAt(cx, cz) + 0.15, cz);
    const occupied = this.tiles.kindAt(gx, gz) !== null;
    const buildable = isBuildableCell(cx, cz);
    const valid =
      this.tool === "bulldoze" ? occupied : this.tool === "pan" ? false : !occupied && buildable;
    (this.cursor.material as THREE.MeshBasicMaterial).color.setHex(
      valid ? 0x8effa0 : 0xff6b5a,
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
    } else if (!occupied && isBuildableCell(cellCenterX(gx), cellCenterZ(gz))) {
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

      const ctx = this.engine.tick(dt, this.camera);

      // Advance the day/night cycle; the sky keeps the sun + its shadow
      // frustum over the camera focus for crisp shadows on the big map.
      this.sky.focus.copy(this.rig.target);
      this.sky.update(ctx);

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
