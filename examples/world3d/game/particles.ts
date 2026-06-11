/**
 * Pooled particle effects.
 *
 * One THREE.Points cloud (fixed capacity) renders every particle in
 * the game — impact dust, muzzle smoke, explosion fireballs, water
 * splashes — in a single draw call. Particles are simulated on the
 * CPU (cheap at this scale) and dead slots are recycled in place;
 * nothing allocates per frame.
 *
 * Tracers are a separate small pool of line segments that stretch
 * from muzzle to hit point and fade over ~80 ms.
 */
import * as THREE from "three";
import type { EventBus, FrameCtx, GameSystem } from "./engine";

const MAX_PARTICLES = 1024;
const MAX_TRACERS = 24;

const PARTICLE_VERT = /* glsl */ `
  attribute float aSize;
  attribute vec3 aColor;
  attribute float aAlpha;
  varying vec3 vColor;
  varying float vAlpha;

  void main() {
    vColor = aColor;
    vAlpha = aAlpha;
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_PointSize = aSize * (220.0 / max(1.0, -mv.z));
    gl_Position = projectionMatrix * mv;
  }
`;

const PARTICLE_FRAG = /* glsl */ `
  varying vec3 vColor;
  varying float vAlpha;

  void main() {
    // Soft round sprite, no texture fetch.
    vec2 d = gl_PointCoord - 0.5;
    float r = length(d) * 2.0;
    float a = (1.0 - smoothstep(0.6, 1.0, r)) * vAlpha;
    if (a < 0.01) discard;
    gl_FragColor = vec4(vColor, a);
  }
`;

interface ParticleSpawn {
  position: THREE.Vector3;
  count: number;
  color: THREE.ColorRepresentation;
  /** Random velocity magnitude range. */
  speed: [number, number];
  /** Upward bias added to the random direction. */
  up?: number;
  size?: [number, number];
  life?: [number, number];
  gravity?: number;
  drag?: number;
}

export class Particles implements GameSystem {
  readonly name = "particles";
  readonly points: THREE.Points;
  readonly tracerGroup = new THREE.Group();

  // Structure-of-arrays particle state.
  private readonly pos: Float32Array;
  private readonly vel = new Float32Array(MAX_PARTICLES * 3);
  private readonly life = new Float32Array(MAX_PARTICLES); // remaining seconds
  private readonly maxLife = new Float32Array(MAX_PARTICLES);
  private readonly gravity = new Float32Array(MAX_PARTICLES);
  private readonly drag = new Float32Array(MAX_PARTICLES);
  private readonly baseSize = new Float32Array(MAX_PARTICLES);
  private readonly sizeAttr: THREE.BufferAttribute;
  private readonly colorAttr: THREE.BufferAttribute;
  private readonly alphaAttr: THREE.BufferAttribute;
  private readonly posAttr: THREE.BufferAttribute;
  private cursor = 0;

  private readonly tracers: Array<{ line: THREE.Line; ttl: number }> = [];
  private readonly tmpColor = new THREE.Color();

  constructor(events: EventBus) {
    const geo = new THREE.BufferGeometry();
    this.pos = new Float32Array(MAX_PARTICLES * 3);
    this.posAttr = new THREE.BufferAttribute(this.pos, 3).setUsage(THREE.DynamicDrawUsage);
    this.sizeAttr = new THREE.BufferAttribute(new Float32Array(MAX_PARTICLES), 1).setUsage(THREE.DynamicDrawUsage);
    this.colorAttr = new THREE.BufferAttribute(new Float32Array(MAX_PARTICLES * 3), 3).setUsage(THREE.DynamicDrawUsage);
    this.alphaAttr = new THREE.BufferAttribute(new Float32Array(MAX_PARTICLES), 1).setUsage(THREE.DynamicDrawUsage);
    geo.setAttribute("position", this.posAttr);
    geo.setAttribute("aSize", this.sizeAttr);
    geo.setAttribute("aColor", this.colorAttr);
    geo.setAttribute("aAlpha", this.alphaAttr);

    const mat = new THREE.ShaderMaterial({
      vertexShader: PARTICLE_VERT,
      fragmentShader: PARTICLE_FRAG,
      transparent: true,
      depthWrite: false,
    });

    this.points = new THREE.Points(geo, mat);
    this.points.frustumCulled = false;
    this.points.renderOrder = 3;

    // Tracer pool: reusable 2-point lines.
    const tracerMat = new THREE.LineBasicMaterial({
      color: "#ffd9a0",
      transparent: true,
      opacity: 0.9,
    });
    for (let i = 0; i < MAX_TRACERS; i++) {
      const g = new THREE.BufferGeometry();
      g.setAttribute(
        "position",
        new THREE.BufferAttribute(new Float32Array(6), 3).setUsage(THREE.DynamicDrawUsage),
      );
      const line = new THREE.Line(g, tracerMat.clone());
      line.visible = false;
      line.frustumCulled = false;
      this.tracerGroup.add(line);
      this.tracers.push({ line, ttl: 0 });
    }

    // React to game events.
    events.on("impact", ({ point, surface }) => {
      if (surface === "water") {
        this.spawn({ position: point, count: 10, color: "#cfe8f0", speed: [2, 5], up: 4, size: [4, 8], life: [0.3, 0.6], gravity: 9 });
      } else if (surface === "building") {
        this.spawn({ position: point, count: 8, color: "#b9b2a4", speed: [1.5, 5], up: 2, size: [3, 7], life: [0.25, 0.55], gravity: 7 });
      } else {
        this.spawn({ position: point, count: 7, color: "#8a7a55", speed: [1, 4], up: 2.5, size: [3, 6], life: [0.3, 0.6], gravity: 6 });
      }
    });
    events.on("explosion", ({ point, radius }) => {
      this.spawn({ position: point, count: 26, color: "#ffb347", speed: [4, 14], up: 3, size: [6, 13], life: [0.25, 0.5], gravity: -2, drag: 4 });
      this.spawn({ position: point, count: 30, color: "#54504a", speed: [3, 10], up: 5, size: [5, 11], life: [0.7, 1.6], gravity: 2, drag: 2 });
      this.spawn({ position: point, count: 14, color: "#ffe9c4", speed: [8, radius * 6], up: 4, size: [2, 4], life: [0.4, 0.9], gravity: 14 });
    });
    events.on("shot", ({ origin, direction }) => {
      // Muzzle puff just in front of the camera.
      const p = origin.clone().addScaledVector(direction, 0.7);
      this.spawn({ position: p, count: 2, color: "#d8d2c4", speed: [0.4, 1.2], up: 0.6, size: [2, 4], life: [0.15, 0.3], gravity: -1 });
    });
  }

  /** Spawn a burst. Overwrites the oldest slots when the pool wraps. */
  spawn(opts: ParticleSpawn) {
    const color = this.tmpColor.set(opts.color);
    const [sMin, sMax] = opts.speed;
    const [szMin, szMax] = opts.size ?? [3, 6];
    const [lMin, lMax] = opts.life ?? [0.4, 0.8];
    for (let n = 0; n < opts.count; n++) {
      const i = this.cursor;
      this.cursor = (this.cursor + 1) % MAX_PARTICLES;

      const theta = Math.random() * Math.PI * 2;
      const phi = Math.acos(2 * Math.random() - 1);
      const speed = sMin + Math.random() * (sMax - sMin);
      this.pos[i * 3] = opts.position.x;
      this.pos[i * 3 + 1] = opts.position.y;
      this.pos[i * 3 + 2] = opts.position.z;
      this.vel[i * 3] = Math.sin(phi) * Math.cos(theta) * speed;
      this.vel[i * 3 + 1] = Math.cos(phi) * speed + (opts.up ?? 0);
      this.vel[i * 3 + 2] = Math.sin(phi) * Math.sin(theta) * speed;
      this.maxLife[i] = lMin + Math.random() * (lMax - lMin);
      this.life[i] = this.maxLife[i];
      this.gravity[i] = opts.gravity ?? 8;
      this.drag[i] = opts.drag ?? 0.6;
      this.baseSize[i] = szMin + Math.random() * (szMax - szMin);
      this.colorAttr.setXYZ(i, color.r, color.g, color.b);
    }
  }

  /** Fire a tracer from a to b. No-op when the pool is saturated. */
  tracer(a: THREE.Vector3, b: THREE.Vector3) {
    const t = this.tracers.find((x) => x.ttl <= 0);
    if (!t) return;
    const posAttr = t.line.geometry.attributes.position as THREE.BufferAttribute;
    posAttr.setXYZ(0, a.x, a.y, a.z);
    posAttr.setXYZ(1, b.x, b.y, b.z);
    posAttr.needsUpdate = true;
    t.ttl = 0.08;
    t.line.visible = true;
    (t.line.material as THREE.LineBasicMaterial).opacity = 0.9;
  }

  update(ctx: FrameCtx) {
    const dt = ctx.dt;
    for (let i = 0; i < MAX_PARTICLES; i++) {
      if (this.life[i] <= 0) {
        if (this.alphaAttr.getX(i) !== 0) this.alphaAttr.setX(i, 0);
        continue;
      }
      this.life[i] -= dt;
      const dragFactor = Math.max(0, 1 - this.drag[i] * dt);
      this.vel[i * 3] *= dragFactor;
      this.vel[i * 3 + 1] = this.vel[i * 3 + 1] * dragFactor - this.gravity[i] * dt;
      this.vel[i * 3 + 2] *= dragFactor;
      this.pos[i * 3] += this.vel[i * 3] * dt;
      this.pos[i * 3 + 1] += this.vel[i * 3 + 1] * dt;
      this.pos[i * 3 + 2] += this.vel[i * 3 + 2] * dt;

      const lifeT = Math.max(0, this.life[i] / this.maxLife[i]);
      this.alphaAttr.setX(i, lifeT * 0.85);
      // Smoke-style growth as particles age.
      this.sizeAttr.setX(i, this.baseSize[i] * (1.6 - lifeT * 0.6));
    }
    this.posAttr.needsUpdate = true;
    this.sizeAttr.needsUpdate = true;
    this.alphaAttr.needsUpdate = true;
    this.colorAttr.needsUpdate = true;

    for (const t of this.tracers) {
      if (t.ttl <= 0) continue;
      t.ttl -= dt;
      if (t.ttl <= 0) {
        t.line.visible = false;
      } else {
        (t.line.material as THREE.LineBasicMaterial).opacity = (t.ttl / 0.08) * 0.9;
      }
    }
  }

  dispose() {
    this.points.geometry.dispose();
    (this.points.material as THREE.Material).dispose();
    for (const t of this.tracers) {
      t.line.geometry.dispose();
      (t.line.material as THREE.Material).dispose();
    }
  }
}
