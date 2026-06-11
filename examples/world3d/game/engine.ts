/**
 * Mini game-engine core.
 *
 * Deliberately small and dependency-free:
 *   - GameSystem  — anything that updates once per frame. The Engine
 *     owns the ordered system list and the frame clock.
 *   - EventBus    — typed pub/sub decoupling systems (weapon fires →
 *     particles + net + buildings react without imports of each other).
 *   - Pool        — fixed-capacity object pool; gameplay allocates
 *     nothing per frame once warmed up.
 *
 * Rendering stays plain three.js — the engine only structures update
 * order, lifetime, and cross-system communication.
 */
import type * as THREE from "three";

/** Per-frame context handed to every system. */
export interface FrameCtx {
  /** Clamped delta time in seconds (≤ 50 ms so tab-switches don't warp physics). */
  dt: number;
  /** Seconds since engine start. */
  time: number;
  camera: THREE.PerspectiveCamera;
  /** Player eye position in world space. */
  playerPos: THREE.Vector3;
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
  /** A hitscan shot connected with the world. */
  impact: { point: THREE.Vector3; normal: THREE.Vector3; surface: "terrain" | "building" | "water" };
  /** The local player fired (muzzle flash, tracer, recoil already applied). */
  shot: { origin: THREE.Vector3; direction: THREE.Vector3 };
  /** A grenade detonated. */
  explosion: { point: THREE.Vector3; radius: number };
  /** The local player lobbed a grenade (broadcast to peers as VFX). */
  grenadeThrown: { origin: THREE.Vector3; velocity: THREE.Vector3 };
  /** Building blocks got destroyed locally (shooter side). Keys must sync. */
  blocksDestroyed: { keys: string[] };
  /** Camera shake request, magnitude in radians-ish. */
  shake: { strength: number };
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
// Pool
// ---------------------------------------------------------------------------

/**
 * Fixed-capacity object pool. `acquire` returns null when exhausted —
 * callers drop the effect rather than allocating (correct behavior for
 * particles/debris under load).
 */
export class Pool<T> {
  private free: T[] = [];
  private readonly live = new Set<T>();

  constructor(create: (index: number) => T, capacity: number) {
    for (let i = 0; i < capacity; i++) this.free.push(create(i));
  }

  acquire(): T | null {
    const item = this.free.pop();
    if (item === undefined) return null;
    this.live.add(item);
    return item;
  }

  release(item: T): void {
    if (!this.live.delete(item)) return; // double-release guard
    this.free.push(item);
  }

  /** Iterate live items; safe to release() the current item mid-loop. */
  forEachLive(fn: (item: T) => void): void {
    for (const item of Array.from(this.live)) fn(item);
  }

  get liveCount(): number {
    return this.live.size;
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

  tick(rawDtSeconds: number, camera: THREE.PerspectiveCamera, playerPos: THREE.Vector3): FrameCtx {
    const dt = Math.min(0.05, Math.max(0, rawDtSeconds));
    this.elapsed += dt;
    const ctx: FrameCtx = { dt, time: this.elapsed, camera, playerPos };
    for (const system of this.systems) system.update(ctx);
    return ctx;
  }

  dispose(): void {
    // Dispose in reverse registration order (dependents first).
    for (let i = this.systems.length - 1; i >= 0; i--) this.systems[i].dispose?.();
    this.systems = [];
    this.events.clear();
  }
}
