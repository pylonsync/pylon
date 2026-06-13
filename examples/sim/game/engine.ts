/**
 * Mini engine core — same shape as the world3d example, trimmed for a
 * city builder (no player, no physics):
 *   - GameSystem  — anything that updates once per frame. The Engine
 *     owns the ordered system list and the frame clock.
 *   - EventBus    — typed pub/sub decoupling systems (paint tool →
 *     net + tile renderer react without importing each other).
 *
 * Rendering stays plain three.js — the engine only structures update
 * order, lifetime, and cross-system communication.
 */
import type * as THREE from "three";

/** A grid cell coordinate (column, row). */
export interface Cell {
  gx: number;
  gz: number;
}

/** Zone / road kinds painted onto the grid. */
export type TileKind = "road" | "res" | "com" | "ind";

/** Per-frame context handed to every system. */
export interface FrameCtx {
  /** Clamped delta time in seconds (≤ 50 ms so tab-switches don't warp anims). */
  dt: number;
  /** Seconds since engine start. */
  time: number;
  camera: THREE.PerspectiveCamera;
}

export interface GameSystem {
  /** Stable name for debugging/profiling. */
  readonly name: string;
  update(ctx: FrameCtx): void;
  dispose?(): void;
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/** All cross-system events. Add new events here — one source of truth. */
export interface GameEvents {
  /** The local player painted cells with the active tool. → net. */
  tilesPainted: { kind: TileKind; cells: Cell[] };
  /** The local player bulldozed cells. → net. */
  tilesBulldozed: { cells: Cell[] };
  /** A building grew a level (or first appeared) at a world point. → VFX. */
  buildingRose: { point: THREE.Vector3 };
  /** A tile was bulldozed at a world point. → dust VFX. */
  tileCleared: { point: THREE.Vector3 };
}

type Handler<T> = (payload: T) => void;

export class EventBus {
  private handlers = new Map<keyof GameEvents, Set<Handler<never>>>();

  on<K extends keyof GameEvents>(event: K, handler: Handler<GameEvents[K]>): () => void {
    let set = this.handlers.get(event);
    if (!set) {
      set = new Set();
      this.handlers.set(event, set);
    }
    set.add(handler as Handler<never>);
    return () => set!.delete(handler as Handler<never>);
  }

  emit<K extends keyof GameEvents>(event: K, payload: GameEvents[K]): void {
    const set = this.handlers.get(event);
    if (!set) return;
    for (const handler of set) (handler as Handler<GameEvents[K]>)(payload);
  }

  clear(): void {
    this.handlers.clear();
  }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

export class Engine {
  readonly events = new EventBus();
  private systems: GameSystem[] = [];
  private elapsed = 0;

  /** Registration order = update order. */
  add<S extends GameSystem>(system: S): S {
    this.systems.push(system);
    return system;
  }

  tick(rawDtSeconds: number, camera: THREE.PerspectiveCamera): FrameCtx {
    const dt = Math.min(0.05, Math.max(0, rawDtSeconds));
    this.elapsed += dt;
    const ctx: FrameCtx = { dt, time: this.elapsed, camera };
    for (const system of this.systems) system.update(ctx);
    return ctx;
  }

  dispose(): void {
    for (let i = this.systems.length - 1; i >= 0; i--) this.systems[i].dispose?.();
    this.systems = [];
    this.events.clear();
  }
}
