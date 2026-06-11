/**
 * Sky dome, sun, clouds, and scene lighting.
 *
 * Sky: one inverted sphere with a gradient shader (zenith → horizon
 * haze) plus an analytic sun disc and glow — no textures.
 *
 * Clouds: a single InstancedMesh of camera-facing quads with a soft
 * procedural puff texture, drifting with the wind and wrapping around
 * the world. One draw call.
 */
import * as THREE from "three";
import { QUALITY, SEED, WORLD_SIZE, type QualityPreset } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import { makeRng } from "./prng";

const SKY_VERT = /* glsl */ `
  varying vec3 vDir;
  void main() {
    vDir = normalize(position);
    vec4 pos = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
    gl_Position = pos.xyww; // force depth = far plane
  }
`;

const SKY_FRAG = /* glsl */ `
  uniform vec3 uSunDir;
  uniform vec3 uHazeColor;
  varying vec3 vDir;

  void main() {
    vec3 dir = normalize(vDir);
    float h = clamp(dir.y, -1.0, 1.0);

    vec3 zenith = vec3(0.17, 0.36, 0.66);
    vec3 horizon = uHazeColor; // matches scene fog → terrain melts into sky
    vec3 below = uHazeColor * 0.92;

    // Haze hugs the horizon; the upper sky stays properly blue.
    vec3 color = h >= 0.0
      ? mix(horizon, zenith, pow(h, 0.6))
      : mix(horizon, below, clamp(-h * 3.0, 0.0, 1.0));

    vec3 sun = normalize(uSunDir);
    float cosAngle = dot(dir, sun);

    // Forward-scatter glow concentrated around the sun (and weighted
    // toward the horizon), then a tighter halo + disc.
    float scatter = pow(max(cosAngle, 0.0), 6.0) * 0.30 * (1.0 - h * 0.75);
    float glow = pow(max(cosAngle, 0.0), 28.0) * 0.5;
    float disc = smoothstep(0.9995, 0.9999, cosAngle);
    color += vec3(1.0, 0.93, 0.80) * scatter;
    color += vec3(1.0, 0.92, 0.74) * glow;
    color += vec3(1.0, 0.97, 0.88) * disc * 6.0;

    gl_FragColor = vec4(color, 1.0);
  }
`;

const CLOUD_VERT = /* glsl */ `
  uniform float uTime;
  uniform float uWindX;
  uniform float uWorldWrap;
  attribute float aScale;
  attribute float aPhase;
  varying vec2 vUv;
  varying float vFade;

  void main() {
    vUv = uv;
    // Instance origin from the instance matrix; drift along +x, wrap.
    vec3 origin = vec3(instanceMatrix[3][0], instanceMatrix[3][1], instanceMatrix[3][2]);
    float span = uWorldWrap * 2.0;
    origin.x = mod(origin.x + uTime * uWindX + uWorldWrap, span) - uWorldWrap;
    origin.y += sin(uTime * 0.05 + aPhase) * 4.0;

    // Billboard: expand the quad in camera space.
    vec3 camRight = vec3(viewMatrix[0][0], viewMatrix[1][0], viewMatrix[2][0]);
    vec3 camUp = vec3(viewMatrix[0][1], viewMatrix[1][1], viewMatrix[2][1]);
    vec3 world = origin
      + camRight * position.x * aScale
      + camUp * position.y * aScale * 0.3;

    // Fade puffs that drift near the wrap seam so they don't pop.
    float edge = abs(origin.x) / uWorldWrap;
    vFade = 1.0 - smoothstep(0.92, 1.0, edge);

    gl_Position = projectionMatrix * viewMatrix * vec4(world, 1.0);
  }
`;

const CLOUD_FRAG = /* glsl */ `
  uniform sampler2D uPuff;
  varying vec2 vUv;
  varying float vFade;

  void main() {
    float a = texture2D(uPuff, vUv).a * 0.62 * vFade;
    if (a < 0.01) discard;
    gl_FragColor = vec4(vec3(1.0), a);
  }
`;

/** Soft radial puff sprite, generated once on a canvas. */
function makePuffTexture(): THREE.Texture {
  const size = 128;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d")!;
  const grad = ctx.createRadialGradient(size / 2, size / 2, 4, size / 2, size / 2, size / 2);
  grad.addColorStop(0, "rgba(255,255,255,0.9)");
  grad.addColorStop(0.55, "rgba(255,255,255,0.45)");
  grad.addColorStop(1, "rgba(255,255,255,0)");
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, size, size);
  const tex = new THREE.CanvasTexture(canvas);
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

export class Sky implements GameSystem {
  readonly name = "sky";
  readonly dome: THREE.Mesh;
  readonly clouds: THREE.InstancedMesh;
  readonly sun: THREE.DirectionalLight;
  readonly ambient: THREE.HemisphereLight;
  // Late-afternoon sun: low elevation → long shadows, warm light,
  // big forward-scatter glow near the horizon.
  readonly sunDir = new THREE.Vector3(0.5, 0.42, 0.36).normalize();

  private readonly cloudUniforms: Record<string, THREE.IUniform>;
  private readonly puffTex: THREE.Texture;
  private readonly shadowSnap: number;

  constructor(scene: THREE.Scene, hazeColor: THREE.Color, quality: QualityPreset = QUALITY.high) {
    // --- Dome ---
    const domeGeo = new THREE.SphereGeometry(900, 32, 16);
    const domeMat = new THREE.ShaderMaterial({
      vertexShader: SKY_VERT,
      fragmentShader: SKY_FRAG,
      uniforms: { uSunDir: { value: this.sunDir }, uHazeColor: { value: hazeColor } },
      side: THREE.BackSide,
      depthWrite: false,
    });
    this.dome = new THREE.Mesh(domeGeo, domeMat);
    this.dome.frustumCulled = false;
    scene.add(this.dome);

    // --- Lights ---
    this.sun = new THREE.DirectionalLight(0xffe3bd, 2.4);
    this.sun.position.copy(this.sunDir).multiplyScalar(180);
    this.sun.castShadow = true;
    this.sun.shadow.mapSize.set(quality.shadowMapSize, quality.shadowMapSize);
    // Tight frustum that follows the player (see update()) — shadows
    // only need to be crisp near the camera.
    const s = 55;
    this.sun.shadow.camera.left = -s;
    this.sun.shadow.camera.right = s;
    this.sun.shadow.camera.top = s;
    this.sun.shadow.camera.bottom = -s;
    this.sun.shadow.camera.near = 20;
    this.sun.shadow.camera.far = 420;
    this.sun.shadow.bias = -0.0008;
    this.shadowSnap = (s * 2) / quality.shadowMapSize;
    scene.add(this.sun);
    scene.add(this.sun.target);

    this.ambient = new THREE.HemisphereLight(0xb6cfe4, 0x5d6844, 0.8);
    scene.add(this.ambient);

    // --- Clouds ---
    this.puffTex = makePuffTexture();
    this.cloudUniforms = {
      uTime: { value: 0 },
      uWindX: { value: 3.2 },
      uWorldWrap: { value: WORLD_SIZE * 1.4 },
      uPuff: { value: this.puffTex },
    };
    const quad = new THREE.PlaneGeometry(1, 1);
    const cloudMat = new THREE.ShaderMaterial({
      vertexShader: CLOUD_VERT,
      fragmentShader: CLOUD_FRAG,
      uniforms: this.cloudUniforms,
      transparent: true,
      depthWrite: false,
    });

    const count = quality.cloudCount;
    this.clouds = new THREE.InstancedMesh(quad, cloudMat, count);
    this.clouds.frustumCulled = false;
    this.clouds.renderOrder = 2;

    const rng = makeRng(SEED ^ 0xc10d);
    const scales = new Float32Array(count);
    const phases = new Float32Array(count);
    const m = new THREE.Matrix4();
    for (let i = 0; i < count; i++) {
      m.setPosition(
        rng.range(-WORLD_SIZE * 1.4, WORLD_SIZE * 1.4),
        rng.range(150, 280),
        rng.range(-WORLD_SIZE * 1.2, WORLD_SIZE * 1.2),
      );
      this.clouds.setMatrixAt(i, m);
      scales[i] = rng.range(90, 260);
      phases[i] = rng.range(0, Math.PI * 2);
    }
    quad.setAttribute("aScale", new THREE.InstancedBufferAttribute(scales, 1));
    quad.setAttribute("aPhase", new THREE.InstancedBufferAttribute(phases, 1));
    this.clouds.instanceMatrix.needsUpdate = true;
    scene.add(this.clouds);
  }

  /** Follow the player: dome recenters, shadow frustum tracks. */
  update(ctx: FrameCtx) {
    const playerPos = ctx.playerPos;
    this.cloudUniforms.uTime.value = ctx.time;
    this.dome.position.copy(playerPos);

    // Snap the shadow camera to texel-sized steps to stop shimmer.
    const snap = this.shadowSnap;
    const tx = Math.round(playerPos.x / snap) * snap;
    const tz = Math.round(playerPos.z / snap) * snap;
    this.sun.position.set(tx, 0, tz).addScaledVector(this.sunDir, 180);
    this.sun.target.position.set(tx, 0, tz);
  }

  dispose() {
    this.dome.geometry.dispose();
    (this.dome.material as THREE.Material).dispose();
    this.clouds.geometry.dispose();
    (this.clouds.material as THREE.Material).dispose();
    this.clouds.dispose();
    this.puffTex.dispose();
    this.sun.dispose();
    this.ambient.dispose();
  }
}
