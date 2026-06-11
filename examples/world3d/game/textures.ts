/**
 * Procedural textures, generated once at boot on canvases — no asset
 * downloads, no network. Flat-shaded geometry is what makes a scene
 * read as "vector art"; these detail maps are the cheapest possible
 * fix: albedo variation for terrain/rock/concrete, fiber for bark, and
 * an alpha-cutout leaflet sheet that turns solid frond quads into
 * feathered palm silhouettes.
 */
import * as THREE from "three";
import { mulberry32 } from "./prng";
import { SEED } from "./config";

function makeCanvas(size: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d")!;
  return [canvas, ctx];
}

function toTexture(canvas: HTMLCanvasElement, repeat: number): THREE.CanvasTexture {
  const tex = new THREE.CanvasTexture(canvas);
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
  tex.repeat.set(repeat, repeat);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = 4;
  return tex;
}

/**
 * Tileable multi-octave value noise rendered as a light-neutral
 * grayscale. Multiplied over vertex colors / material color it adds
 * the ground-clutter speckle that kills the "flat vector" look.
 */
export function makeDetailTexture(repeat: number, contrast = 0.22): THREE.CanvasTexture {
  const size = 256;
  const [canvas, ctx] = makeCanvas(size);
  const rand = mulberry32(SEED ^ 0x7e57);

  // Tileable value noise: random grid, bilinear sample with wrapping.
  const makeGrid = (n: number) => {
    const g = new Float32Array(n * n);
    for (let i = 0; i < g.length; i++) g[i] = rand();
    return g;
  };
  const sample = (g: Float32Array, n: number, x: number, y: number) => {
    const gx = Math.floor(x) % n;
    const gy = Math.floor(y) % n;
    const fx = x - Math.floor(x);
    const fy = y - Math.floor(y);
    const sx = fx * fx * (3 - 2 * fx);
    const sy = fy * fy * (3 - 2 * fy);
    const a = g[gy * n + gx];
    const b = g[gy * n + ((gx + 1) % n)];
    const c = g[((gy + 1) % n) * n + gx];
    const d = g[((gy + 1) % n) * n + ((gx + 1) % n)];
    return a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy;
  };

  const octaves = [
    { grid: makeGrid(8), n: 8, amp: 0.5 },
    { grid: makeGrid(16), n: 16, amp: 0.3 },
    { grid: makeGrid(32), n: 32, amp: 0.2 },
  ];

  const img = ctx.createImageData(size, size);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let v = 0;
      for (const o of octaves) {
        v += sample(o.grid, o.n, (x / size) * o.n, (y / size) * o.n) * o.amp;
      }
      // Centered around 1.0 in multiply terms: 255 * (1 - contrast/2 + v*contrast)
      const lum = Math.round(255 * (1 - contrast / 2 + (v - 0.5) * contrast));
      const i = (y * size + x) * 4;
      img.data[i] = img.data[i + 1] = img.data[i + 2] = lum;
      img.data[i + 3] = 255;
    }
  }
  ctx.putImageData(img, 0, 0);
  return toTexture(canvas, repeat);
}

/**
 * Palm-frond leaflet sheet with alpha. The frond geometry is a long
 * quad; this texture draws a center rib with angled leaflets and
 * transparent gaps, so alphaTest carves a feathered silhouette.
 */
export function makeFrondTexture(): THREE.CanvasTexture {
  const w = 256;
  const h = 64;
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d")!;
  const rand = mulberry32(SEED ^ 0xf20d);

  ctx.clearRect(0, 0, w, h);

  // Leaflets: dense overlapping angled strokes, shrinking to the tip.
  // Two passes (shadow pass darker + slightly offset) give depth.
  for (const pass of [0, 1]) {
    for (let x = 4; x < w - 2; x += 3) {
      const t = x / w;
      const len = (h / 2 - 2) * (1 - t * 0.45) * (0.9 + rand() * 0.25);
      const lean = 10 + t * 22; // sweep back toward the tip
      const g = pass === 0 ? 95 + Math.floor(rand() * 40) : 140 + Math.floor(rand() * 55);
      ctx.strokeStyle = `rgba(${Math.floor(g * 0.45)}, ${g}, ${Math.floor(g * 0.4)}, 1)`;
      ctx.lineWidth = pass === 0 ? 4.2 : 3.0;
      const xo = pass === 0 ? 1.5 : 0;
      ctx.beginPath();
      ctx.moveTo(x + xo, h / 2);
      ctx.lineTo(x + xo + lean * 0.4, h / 2 - len);
      ctx.moveTo(x + xo + 1.5, h / 2);
      ctx.lineTo(x + xo + 1.5 + lean * 0.4, h / 2 + len);
      ctx.stroke();
    }
  }

  // Center rib drawn last so it reads over the leaflet roots.
  ctx.strokeStyle = "rgba(120, 140, 80, 1)";
  ctx.lineWidth = 3;
  ctx.beginPath();
  ctx.moveTo(0, h / 2);
  ctx.lineTo(w, h / 2);
  ctx.stroke();

  const tex = new THREE.CanvasTexture(canvas);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = 4;
  return tex;
}

/** Vertical fiber + ring bands for palm trunks. */
export function makeBarkTexture(): THREE.CanvasTexture {
  const size = 128;
  const [canvas, ctx] = makeCanvas(size);
  const rand = mulberry32(SEED ^ 0xba2c);

  ctx.fillStyle = "#8a7458";
  ctx.fillRect(0, 0, size, size);

  // Vertical fibers.
  for (let i = 0; i < 240; i++) {
    const x = rand() * size;
    const y = rand() * size;
    const len = 8 + rand() * 26;
    const shade = 0.75 + rand() * 0.5;
    ctx.strokeStyle = `rgba(${Math.floor(110 * shade)}, ${Math.floor(92 * shade)}, ${Math.floor(70 * shade)}, 0.5)`;
    ctx.lineWidth = 1 + rand();
    ctx.beginPath();
    ctx.moveTo(x, y);
    ctx.lineTo(x + (rand() - 0.5) * 3, y + len);
    ctx.stroke();
  }
  // Ring scars.
  for (let y = 6; y < size; y += 14 + Math.floor(rand() * 8)) {
    ctx.strokeStyle = "rgba(60, 48, 36, 0.35)";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(size, y + (rand() - 0.5) * 4);
    ctx.stroke();
  }
  return toTexture(canvas, 1);
}

/**
 * Grass CLUMP sheet — the technique every jungle shooter uses: a fan
 * of ~14 curved, tapering blades baked into one alpha texture, mapped
 * onto crossed cards. One instance reads as a whole tuft of grass
 * instead of a lone spike, so the field gets visual mass without more
 * geometry. Color is baked here (olive roots → sunlit tips); the
 * shader multiplies baked terrain lighting, tint, and fog on top.
 */
export function makeGrassClumpTexture(): THREE.CanvasTexture {
  const w = 256;
  const h = 128;
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d")!;
  const rand = mulberry32(SEED ^ 0x96a55);

  ctx.clearRect(0, 0, w, h);

  const blades = 16;
  for (let i = 0; i < blades; i++) {
    // Fan out from a root cluster at bottom-center.
    const spread = (i / (blades - 1)) * 2 - 1; // -1 .. 1
    const rootX = w / 2 + spread * 14 + rand() * 6 - 3;
    const height = h * (0.55 + rand() * 0.42);
    const tipX = rootX + spread * (26 + rand() * 38) + (rand() - 0.5) * 12;
    const ctrlX = rootX + (tipX - rootX) * (0.25 + rand() * 0.2);
    const rootW = 3.5 + rand() * 3.5;

    // Blade body: two quadratic edges meeting at the tip.
    const rootY = h;
    const tipY = h - height;
    const ctrlY = h - height * 0.55;
    ctx.beginPath();
    ctx.moveTo(rootX - rootW / 2, rootY);
    ctx.quadraticCurveTo(ctrlX - rootW / 2.6, ctrlY, tipX, tipY);
    ctx.quadraticCurveTo(ctrlX + rootW / 2.6, ctrlY, rootX + rootW / 2, rootY);
    ctx.closePath();

    // Root-to-tip gradient: dark olive base, sunlit khaki-green tip.
    const grad = ctx.createLinearGradient(0, rootY, 0, tipY);
    const v = 0.85 + rand() * 0.3;
    grad.addColorStop(0, `rgb(${Math.round(34 * v)}, ${Math.round(48 * v)}, ${Math.round(20 * v)})`);
    grad.addColorStop(0.6, `rgb(${Math.round(64 * v)}, ${Math.round(86 * v)}, ${Math.round(32 * v)})`);
    grad.addColorStop(1, `rgb(${Math.round(108 * v)}, ${Math.round(124 * v)}, ${Math.round(52 * v)})`);
    ctx.fillStyle = grad;
    ctx.fill();
  }

  const tex = new THREE.CanvasTexture(canvas);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = 4;
  return tex;
}

/**
 * Wide oval leaf with a center vein and side veins — alpha outside the
 * leaf shape. Used by the broadleaf ground plants with alphaTest.
 */
export function makeLeafTexture(): THREE.CanvasTexture {
  const w = 128;
  const h = 64;
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d")!;

  ctx.clearRect(0, 0, w, h);

  // Leaf body: pointed oval from root (left) to tip (right).
  ctx.fillStyle = "rgba(95, 150, 70, 1)";
  ctx.beginPath();
  ctx.moveTo(2, h / 2);
  ctx.bezierCurveTo(w * 0.25, 2, w * 0.75, 4, w - 2, h / 2);
  ctx.bezierCurveTo(w * 0.75, h - 4, w * 0.25, h - 2, 2, h / 2);
  ctx.fill();

  // Slightly darker outer gradient for curl shading.
  const grad = ctx.createLinearGradient(0, 0, 0, h);
  grad.addColorStop(0, "rgba(40, 70, 30, 0.35)");
  grad.addColorStop(0.5, "rgba(0, 0, 0, 0)");
  grad.addColorStop(1, "rgba(40, 70, 30, 0.35)");
  ctx.globalCompositeOperation = "source-atop";
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, w, h);

  // Veins.
  ctx.strokeStyle = "rgba(150, 190, 110, 0.8)";
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.moveTo(2, h / 2);
  ctx.lineTo(w - 4, h / 2);
  ctx.stroke();
  ctx.lineWidth = 1;
  for (let x = 14; x < w - 10; x += 11) {
    ctx.beginPath();
    ctx.moveTo(x, h / 2);
    ctx.lineTo(x + 14, 6 + (x % 3));
    ctx.moveTo(x, h / 2);
    ctx.lineTo(x + 14, h - 6 - (x % 3));
    ctx.stroke();
  }
  ctx.globalCompositeOperation = "source-over";

  const tex = new THREE.CanvasTexture(canvas);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = 4;
  return tex;
}

/**
 * Tangent-space normal map derived from a luminance canvas via Sobel
 * gradients — gives rocks/concrete real surface relief under the sun
 * without any geometry cost.
 */
export function makeNormalMapFromCanvas(source: HTMLCanvasElement, strength: number, repeat: number): THREE.CanvasTexture {
  const size = source.width;
  const srcCtx = source.getContext("2d")!;
  const src = srcCtx.getImageData(0, 0, size, size).data;
  const lum = (x: number, y: number) => {
    const xx = ((x % size) + size) % size;
    const yy = ((y % size) + size) % size;
    const i = (yy * size + xx) * 4;
    return (src[i] + src[i + 1] + src[i + 2]) / (3 * 255);
  };

  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d")!;
  const img = ctx.createImageData(size, size);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      // Sobel gradients (wrapping → the normal map tiles like the albedo).
      const dx =
        lum(x + 1, y - 1) + 2 * lum(x + 1, y) + lum(x + 1, y + 1) -
        (lum(x - 1, y - 1) + 2 * lum(x - 1, y) + lum(x - 1, y + 1));
      const dy =
        lum(x - 1, y + 1) + 2 * lum(x, y + 1) + lum(x + 1, y + 1) -
        (lum(x - 1, y - 1) + 2 * lum(x, y - 1) + lum(x + 1, y - 1));
      const nx = -dx * strength;
      const ny = -dy * strength;
      const nz = 1;
      const len = Math.hypot(nx, ny, nz);
      const i = (y * size + x) * 4;
      img.data[i] = Math.round(((nx / len) * 0.5 + 0.5) * 255);
      img.data[i + 1] = Math.round(((ny / len) * 0.5 + 0.5) * 255);
      img.data[i + 2] = Math.round(((nz / len) * 0.5 + 0.5) * 255);
      img.data[i + 3] = 255;
    }
  }
  ctx.putImageData(img, 0, 0);

  const tex = new THREE.CanvasTexture(canvas);
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
  tex.repeat.set(repeat, repeat);
  tex.colorSpace = THREE.NoColorSpace; // normal data, not color
  tex.anisotropy = 4;
  return tex;
}

/** Blotchy mineral noise for rocks and concrete (tinted by material color). */
export function makeRockTexture(repeat: number): THREE.CanvasTexture {
  const size = 256;
  const [canvas, ctx] = makeCanvas(size);
  const rand = mulberry32(SEED ^ 0x70c4);

  ctx.fillStyle = "#9a9a98";
  ctx.fillRect(0, 0, size, size);

  // Layered blotches.
  for (let i = 0; i < 900; i++) {
    const x = rand() * size;
    const y = rand() * size;
    const r = 1 + rand() * 7;
    const shade = 0.72 + rand() * 0.5;
    ctx.fillStyle = `rgba(${Math.floor(150 * shade)}, ${Math.floor(150 * shade)}, ${Math.floor(146 * shade)}, ${0.12 + rand() * 0.2})`;
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fill();
  }
  // Cracks.
  for (let i = 0; i < 26; i++) {
    let x = rand() * size;
    let y = rand() * size;
    ctx.strokeStyle = "rgba(55, 55, 52, 0.30)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x, y);
    for (let s = 0; s < 6; s++) {
      x += (rand() - 0.5) * 26;
      y += (rand() - 0.5) * 26;
      ctx.lineTo(x, y);
    }
    ctx.stroke();
  }
  return toTexture(canvas, repeat);
}
