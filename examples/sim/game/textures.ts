/**
 * Canvas-generated textures. Procedural so the example ships with zero
 * image assets — a window grid for building facades and a subtle grid
 * for the ground plane.
 */
import * as THREE from "three";

function canvas(size: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const c = document.createElement("canvas");
  c.width = c.height = size;
  const ctx = c.getContext("2d")!;
  return [c, ctx];
}

/**
 * A tileable building facade: rows of lit/unlit windows over a tinted
 * wall. `warm` lights some windows so towers read as occupied at dusk.
 */
export function makeFacadeTexture(wall: string): THREE.CanvasTexture {
  const [c, ctx] = canvas(128);
  ctx.fillStyle = wall;
  ctx.fillRect(0, 0, 128, 128);
  const cols = 4;
  const rows = 5;
  const pad = 10;
  const cw = (128 - pad * (cols + 1)) / cols;
  const ch = (128 - pad * (rows + 1)) / rows;
  for (let r = 0; r < rows; r++) {
    for (let col = 0; col < cols; col++) {
      const lit = (r * 7 + col * 13) % 5 === 0;
      ctx.fillStyle = lit ? "#ffe9b0" : "#1c2733";
      const x = pad + col * (cw + pad);
      const y = pad + r * (ch + pad);
      ctx.fillRect(x, y, cw, ch);
      // glass sheen
      ctx.fillStyle = "rgba(255,255,255,0.08)";
      ctx.fillRect(x, y, cw, ch * 0.35);
    }
  }
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
  return tex;
}

/**
 * An equirectangular sky gradient for image-based lighting: deep blue
 * zenith → pale horizon → greenish ground bounce. PMREM'd into the
 * scene environment, this gives outdoor materials natural directional
 * ambient (cool from the sky, warm/green from the ground) instead of a
 * flat studio look.
 */
export function makeSkyEnvTexture(): THREE.CanvasTexture {
  const w = 16;
  const h = 256;
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const ctx = c.getContext("2d")!;
  const g = ctx.createLinearGradient(0, 0, 0, h);
  g.addColorStop(0.0, "#3f74c8"); // zenith
  g.addColorStop(0.4, "#9cc0e4");
  g.addColorStop(0.49, "#dfeaf2"); // horizon haze
  g.addColorStop(0.5, "#cdd2c4"); // horizon line
  g.addColorStop(0.62, "#8f9d77"); // ground near
  g.addColorStop(1.0, "#5d6a48"); // ground far
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, w, h);
  const tex = new THREE.CanvasTexture(c);
  tex.mapping = THREE.EquirectangularReflectionMapping;
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

/**
 * Faint grid for the ground plane so empty land still reads as a
 * planned map. One big repeating texture under everything.
 */
export function makeGroundTexture(): THREE.CanvasTexture {
  const N = 256;
  const [c, ctx] = canvas(N);
  // Layered grass: a mid base, broad tonal patches, fine blade noise and
  // a few dirt flecks so it reads as turf rather than a flat fill. Tiles
  // seamlessly (everything wraps via modulo).
  ctx.fillStyle = "#4f6b39";
  ctx.fillRect(0, 0, N, N);

  const greens = ["#586f3c", "#46612f", "#5f7a42", "#42592c", "#657f47"];
  // Broad soft patches (mowed-grass variation).
  for (let i = 0; i < 90; i++) {
    const r = 18 + Math.random() * 46;
    const x = Math.random() * N;
    const y = Math.random() * N;
    ctx.globalAlpha = 0.18 + Math.random() * 0.2;
    ctx.fillStyle = greens[(Math.random() * greens.length) | 0];
    for (let dx = -N; dx <= N; dx += N)
      for (let dy = -N; dy <= N; dy += N) {
        ctx.beginPath();
        ctx.arc(x + dx, y + dy, r, 0, Math.PI * 2);
        ctx.fill();
      }
  }
  ctx.globalAlpha = 1;
  // Fine blade noise.
  for (let i = 0; i < 4200; i++) {
    const x = Math.random() * N;
    const y = Math.random() * N;
    const dark = Math.random() < 0.5;
    ctx.fillStyle = dark ? "rgba(20,40,16,0.18)" : "rgba(150,180,110,0.16)";
    ctx.fillRect(x, y, 1, Math.random() < 0.5 ? 1 : 2);
  }
  // Sparse dirt flecks.
  for (let i = 0; i < 70; i++) {
    ctx.fillStyle = `rgba(120,96,60,${0.1 + Math.random() * 0.18})`;
    ctx.beginPath();
    ctx.arc(Math.random() * N, Math.random() * N, 1 + Math.random() * 2.5, 0, Math.PI * 2);
    ctx.fill();
  }

  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
  tex.anisotropy = 8;
  return tex;
}
