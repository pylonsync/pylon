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
 * Faint grid for the ground plane so empty land still reads as a
 * planned map. One big repeating texture under everything.
 */
export function makeGroundTexture(): THREE.CanvasTexture {
  const [c, ctx] = canvas(64);
  // Light grass-green turf so the map reads as buildable land.
  ctx.fillStyle = "#7c9266";
  ctx.fillRect(0, 0, 64, 64);
  // soft mottling
  for (let i = 0; i < 70; i++) {
    const g = Math.random() < 0.5 ? 255 : 0;
    ctx.fillStyle = `rgba(${g},${g},${g},${Math.random() * 0.05})`;
    ctx.fillRect(Math.random() * 64, Math.random() * 64, 2, 2);
  }
  // Per-cell grid line.
  ctx.strokeStyle = "rgba(255,255,255,0.12)";
  ctx.lineWidth = 1;
  ctx.strokeRect(0.5, 0.5, 63, 63);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
  return tex;
}
