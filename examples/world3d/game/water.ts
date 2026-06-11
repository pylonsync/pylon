/**
 * Animated ocean plane. One quad grid with a custom shader:
 *   - vertex: two crossed gerstner-ish sine bands displace y
 *   - fragment: depth tint from the terrain heightmap, fresnel toward
 *     the horizon, sun glint, and animated shoreline foam
 * No reflection render targets — the whole ocean is a single cheap
 * draw call that still reads convincingly at gameplay distances.
 */
import * as THREE from "three";
import { WATER_LEVEL, WORLD_SIZE } from "./config";
import type { FrameCtx, GameSystem } from "./engine";
import type { Terrain } from "./terrain";

const VERT = /* glsl */ `
  uniform float uTime;
  varying vec3 vWorldPos;
  varying float vWave;

  void main() {
    vec4 wp = modelMatrix * vec4(position, 1.0);
    float w1 = sin(wp.x * 0.13 + uTime * 1.1) * sin(wp.z * 0.11 - uTime * 0.7);
    float w2 = sin((wp.x + wp.z) * 0.045 + uTime * 0.6);
    float w3 = sin(wp.x * 0.4 - uTime * 1.9) * 0.18;
    float wave = w1 * 0.22 + w2 * 0.3 + w3 * 0.08;
    wp.y += wave;
    vWave = wave;
    vWorldPos = wp.xyz;
    gl_Position = projectionMatrix * viewMatrix * wp;
  }
`;

const FRAG = /* glsl */ `
  uniform float uTime;
  uniform vec3 uSunDir;
  uniform vec3 uCameraPos;
  uniform sampler2D uHeightTex;
  uniform float uWorldSize;
  uniform vec3 uFogColor;
  uniform float uFogDensity;
  varying vec3 vWorldPos;
  varying float vWave;

  // Cheap tiling value noise for surface normal detail.
  float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
  }
  float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(
      mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
      mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x),
      u.y);
  }
  // Two octaves of scrolling noise as a heightfield; derivatives give
  // the detail normal. Layers move in different directions so the
  // surface never reads as a repeating sheet.
  float surf(vec2 p) {
    return vnoise(p * 0.9 + vec2(uTime * 0.06, uTime * 0.03)) * 0.65
         + vnoise(p * 2.3 - vec2(uTime * 0.10, -uTime * 0.07)) * 0.35;
  }

  void main() {
    // Terrain height under this fragment → water depth.
    vec2 uv = vWorldPos.xz / uWorldSize + 0.5;
    float ground = texture2D(uHeightTex, uv).r;
    float depth = clamp((${WATER_LEVEL.toFixed(1)} - ground) / 14.0, 0.0, 1.0);

    // Tropical ramp: white-green wash at the sand line, turquoise
    // shelf, then deep blue past the drop-off.
    vec3 wash = vec3(0.45, 0.80, 0.72);
    vec3 turquoise = vec3(0.07, 0.46, 0.50);
    vec3 deep = vec3(0.012, 0.10, 0.22);
    vec3 color = depth < 0.25
      ? mix(wash, turquoise, depth / 0.25)
      : mix(turquoise, deep, sqrt((depth - 0.25) / 0.75));

    // Detail normal from the scrolling noise heightfield.
    vec2 p = vWorldPos.xz * 0.18;
    float e = 0.08;
    float hC = surf(p);
    float hX = surf(p + vec2(e, 0.0));
    float hZ = surf(p + vec2(0.0, e));
    vec3 normal = normalize(vec3((hC - hX) / e * 0.18, 1.0, (hC - hZ) / e * 0.18));
    // Big swell tilt from the vertex wave on top.
    normal = normalize(normal + vec3(vWave * 0.35, 0.0, vWave * 0.25));

    vec3 viewDir = normalize(uCameraPos - vWorldPos);

    // Fresnel: grazing angles mirror the sky/haze.
    float fresnel = pow(1.0 - max(dot(viewDir, normal), 0.0), 3.0);
    color = mix(color, uFogColor * 1.05, fresnel * 0.7);

    // Sun glints: a tight specular core plus a wide sparkle lobe that
    // the detail normal breaks into glitter.
    vec3 halfDir = normalize(uSunDir + viewDir);
    float ndh = max(dot(normal, halfDir), 0.0);
    color += vec3(1.0, 0.95, 0.8) * pow(ndh, 480.0) * 2.4;
    color += vec3(1.0, 0.9, 0.7) * pow(ndh, 60.0) * 0.25;

    // Shore foam: a bright breaker line right at the waterline plus a
    // softer noise-eaten band behind it, both animated.
    float breaker = smoothstep(0.05, 0.0, depth)
      * (0.6 + 0.4 * sin(ground * 18.0 - uTime * 2.6));
    float band = smoothstep(0.18, 0.02, depth)
      * vnoise(vWorldPos.xz * 0.7 + vec2(uTime * 0.25, -uTime * 0.18));
    float foam = clamp(breaker + band * 0.7, 0.0, 1.0);
    color = mix(color, vec3(0.93, 0.97, 0.97), foam * 0.85);

    // Match the scene's exponential fog so the horizon blends.
    float dist = length(uCameraPos - vWorldPos);
    float fogFactor = 1.0 - exp(-uFogDensity * uFogDensity * dist * dist);
    color = mix(color, uFogColor, fogFactor);

    float alpha = mix(0.72, 0.96, depth);
    gl_FragColor = vec4(color, alpha);
  }
`;

export class Water implements GameSystem {
  readonly name = "water";
  readonly mesh: THREE.Mesh;
  private readonly uniforms: Record<string, THREE.IUniform>;

  constructor(terrain: Terrain, sunDir: THREE.Vector3, fog: THREE.FogExp2) {
    this.uniforms = {
      uTime: { value: 0 },
      uSunDir: { value: sunDir },
      uCameraPos: { value: new THREE.Vector3() },
      uHeightTex: { value: terrain.buildHeightTexture() },
      uWorldSize: { value: WORLD_SIZE },
      uFogColor: { value: fog.color },
      uFogDensity: { value: fog.density },
    };

    // Ocean extends well past the island so the horizon is water.
    const geo = new THREE.PlaneGeometry(WORLD_SIZE * 4, WORLD_SIZE * 4, 96, 96);
    geo.rotateX(-Math.PI / 2);

    const mat = new THREE.ShaderMaterial({
      vertexShader: VERT,
      fragmentShader: FRAG,
      uniforms: this.uniforms,
      transparent: true,
      depthWrite: false,
    });

    this.mesh = new THREE.Mesh(geo, mat);
    this.mesh.position.y = WATER_LEVEL;
    this.mesh.name = "water";
    // The plane is huge; never let frustum culling pop it out.
    this.mesh.frustumCulled = false;
    this.mesh.renderOrder = 1; // after opaque terrain for correct blending
  }

  update(ctx: FrameCtx) {
    this.uniforms.uTime.value = ctx.time;
    (this.uniforms.uCameraPos.value as THREE.Vector3).setFromMatrixPosition(
      ctx.camera.matrixWorld,
    );
  }

  dispose() {
    this.mesh.geometry.dispose();
    (this.mesh.material as THREE.ShaderMaterial).dispose();
    (this.uniforms.uHeightTex.value as THREE.Texture).dispose();
  }
}
