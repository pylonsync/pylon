/**
 * Sky + day/night cycle. Owns the scene's lighting: a gradient sky
 * dome, a sun and a moon disc that orbit, the sun/moon directional
 * lights (the sun casts shadows), and the hemisphere fill. Every frame
 * it advances a time-of-day clock and repaints the sky, fog, light
 * directions, colours and intensities — dawn/day/dusk/night.
 *
 * The sun light's shadow frustum follows `focus` (the camera's ground
 * target) so shadows stay crisp across the big map.
 */
import * as THREE from "three";
import { WORLD_HALF } from "./config";
import type { FrameCtx, GameSystem } from "./engine";

const DOME_R = WORLD_HALF * 4;

const SKY_VERT = /* glsl */ `
  varying vec3 vDir;
  void main() {
    vDir = normalize(position);
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
`;
const SKY_FRAG = /* glsl */ `
  varying vec3 vDir;
  uniform vec3 uTop;
  uniform vec3 uHorizon;
  uniform vec3 uSunDir;
  uniform vec3 uSunColor;
  uniform vec3 uMoonDir;
  void main() {
    float h = clamp(vDir.y, -0.1, 1.0);
    vec3 col = mix(uHorizon, uTop, pow(clamp(h, 0.0, 1.0), 0.55));
    // Sun glow + disc.
    float sd = max(dot(normalize(vDir), normalize(uSunDir)), 0.0);
    col += uSunColor * pow(sd, 220.0) * 1.4;        // disc
    col += uSunColor * pow(sd, 8.0) * 0.18;          // halo
    // Moon glow.
    float md = max(dot(normalize(vDir), normalize(uMoonDir)), 0.0);
    col += vec3(0.8, 0.85, 1.0) * pow(md, 400.0) * 0.7;
    gl_FragColor = vec4(col, 1.0);
  }
`;

const col = (hex: number) => new THREE.Color(hex);
const lerpCol = (a: THREE.Color, b: THREE.Color, t: number) => a.clone().lerp(b, t);

export class Sky implements GameSystem {
  readonly name = "sky";
  readonly sun: THREE.DirectionalLight;
  /** Ground target the sun shadow frustum follows. */
  readonly focus = new THREE.Vector3();

  private readonly moon: THREE.DirectionalLight;
  private readonly hemi: THREE.HemisphereLight;
  private readonly dome: THREE.Mesh;
  private readonly mat: THREE.ShaderMaterial;
  private readonly sunDir = new THREE.Vector3();
  private readonly moonDir = new THREE.Vector3();

  /** 0=midnight, .25=dawn, .5=noon, .75=dusk. Starts mid-morning. */
  private t = 0.32;
  /** Full day length in seconds. */
  private readonly dayLength = 210;

  constructor(private readonly scene: THREE.Scene) {
    this.mat = new THREE.ShaderMaterial({
      uniforms: {
        uTop: { value: col(0x2a6bd0) },
        uHorizon: { value: col(0xaed2ea) },
        uSunDir: { value: new THREE.Vector3(0, 1, 0) },
        uSunColor: { value: col(0xfff4d8) },
        uMoonDir: { value: new THREE.Vector3(0, -1, 0) },
      },
      vertexShader: SKY_VERT,
      fragmentShader: SKY_FRAG,
      side: THREE.BackSide,
      depthWrite: false,
      fog: false,
    });
    this.dome = new THREE.Mesh(new THREE.SphereGeometry(DOME_R, 32, 16), this.mat);
    this.dome.name = "sky";
    scene.add(this.dome);

    this.hemi = new THREE.HemisphereLight(0xcfe0ee, 0x55603f, 1.0);
    scene.add(this.hemi);

    this.sun = new THREE.DirectionalLight(0xffeed0, 2.4);
    this.sun.castShadow = true;
    this.sun.shadow.mapSize.set(2048, 2048);
    this.sun.shadow.camera.near = 1;
    this.sun.shadow.camera.far = 600;
    const s = 90;
    this.sun.shadow.camera.left = -s;
    this.sun.shadow.camera.right = s;
    this.sun.shadow.camera.top = s;
    this.sun.shadow.camera.bottom = -s;
    this.sun.shadow.bias = -0.0005;
    scene.add(this.sun);
    scene.add(this.sun.target);

    this.moon = new THREE.DirectionalLight(0x9fb4d8, 0);
    scene.add(this.moon);
    scene.add(this.moon.target);

    // Closer, brighter fog → atmospheric haze that fades the distant
    // hills and skyline into the sky (the CS depth cue).
    scene.fog = new THREE.Fog(0xcfe2ef, WORLD_HALF * 0.55, WORLD_HALF * 2.4);
  }

  update(ctx: FrameCtx): void {
    this.t = (this.t + ctx.dt / this.dayLength) % 1;

    // Sun orbit: dawn (t=.25) rises east, noon up, dusk (t=.75) west.
    const ang = (this.t - 0.25) * Math.PI * 2;
    const elev = Math.sin(ang); // -1..1 ; >0 = daytime
    const ew = Math.cos(ang);
    this.sunDir.set(ew * 0.9, Math.max(elev, -0.4), 0.35).normalize();
    this.moonDir.copy(this.sunDir).negate();

    const day = smooth(-0.04, 0.22, elev); // 0 night .. 1 day
    const dusk = Math.max(0, 1 - Math.abs(elev) / 0.28) * (1 - day) * 1.0; // warm band near horizon
    const night = 1 - day;

    // --- Sky colours ---
    const top = lerpCol(col(0x070d22), col(0x2a6bd0), day);
    let horizon = lerpCol(col(0x121d33), col(0xaed2ea), day);
    horizon = lerpCol(horizon, col(0xec9050), dusk); // warm dusk/dawn
    (this.mat.uniforms.uTop.value as THREE.Color).copy(top);
    (this.mat.uniforms.uHorizon.value as THREE.Color).copy(horizon);
    const sunCol = lerpCol(col(0xff8a3c), col(0xfff4d8), smooth(0.0, 0.35, elev));
    (this.mat.uniforms.uSunColor.value as THREE.Color).copy(sunCol);
    (this.mat.uniforms.uSunDir.value as THREE.Vector3).copy(this.sunDir);
    (this.mat.uniforms.uMoonDir.value as THREE.Vector3).copy(this.moonDir);

    // --- Lights ---
    this.sun.color.copy(sunCol);
    this.sun.intensity = day * 2.9;
    this.moon.intensity = night * 0.6;
    this.moon.color.set(0x9fb4d8);
    // Lean on the sky env for ambient; keep the hemisphere as a gentle
    // fill so the scene isn't a flat wash. The night FLOORs are lifted a
    // little (a CS night has an ambient city glow, not pitch black) while
    // the day values stay identical — the daytime look is unchanged.
    this.hemi.intensity = 0.3 + day * 0.42;
    this.hemi.color.copy(lerpCol(col(0x32405a), col(0xcfe0ee), day));
    this.hemi.groundColor.copy(lerpCol(col(0x1a2018), col(0x55603f), day));
    this.scene.environmentIntensity = 0.26 + day * 0.47;

    // Haze sits a touch brighter than the horizon so the far hills read
    // as atmospheric depth rather than vanishing into a flat band.
    const haze = lerpCol(horizon, col(0xf2f6f8), 0.32 * day);
    if (this.scene.fog instanceof THREE.Fog) this.scene.fog.color.copy(haze);
    if (this.scene.background instanceof THREE.Color) this.scene.background.copy(horizon);

    // --- Positions: lights from their directions, around the focus ---
    const f = this.focus;
    this.dome.position.set(f.x, 0, f.z);
    this.sun.position.set(f.x + this.sunDir.x * 220, this.sunDir.y * 220, f.z + this.sunDir.z * 220);
    this.sun.target.position.set(f.x, 0, f.z);
    this.moon.position.set(f.x + this.moonDir.x * 220, this.moonDir.y * 220, f.z + this.moonDir.z * 220);
    this.moon.target.position.set(f.x, 0, f.z);
  }

  dispose(): void {
    this.dome.geometry.dispose();
    this.mat.dispose();
  }
}

function smooth(a: number, b: number, x: number): number {
  const t = Math.max(0, Math.min(1, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
}
