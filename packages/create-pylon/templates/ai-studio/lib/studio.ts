// Shared studio types + the no-API-key placeholder generator (pure, so the
// generate action can call it server-side and the client imports only the type).

export type GenerationKind = "image" | "audio" | "video";

export interface GenerationRow {
  id: string;
  userId: string;
  kind: string;
  prompt: string;
  status: string; // "pending" | "done" | "failed"
  resultUrl?: string | null;
  error?: string | null;
  demo: boolean;
  createdAt: string;
}

// Deterministic small hash → drives the placeholder gradient hue.
function hashHue(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return h % 360;
}

function escapeXml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

// A self-contained SVG "image" data URL — used when no REPLICATE_API_TOKEN is
// set, so the gallery + realtime flow work with zero config. Clearly a
// placeholder (it renders the prompt over a gradient), not a fake photo. Returns
// a `data:` URL that drops straight into an <img>.
export function placeholderImage(prompt: string): string {
  const hue = hashHue(prompt);
  const hue2 = (hue + 60) % 360;
  const text = escapeXml(prompt.slice(0, 80));
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="640" viewBox="0 0 640 640">
  <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="hsl(${hue} 70% 55%)"/>
    <stop offset="1" stop-color="hsl(${hue2} 70% 45%)"/>
  </linearGradient></defs>
  <rect width="640" height="640" fill="url(#g)"/>
  <text x="50%" y="44%" fill="rgba(255,255,255,0.95)" font-family="system-ui,sans-serif" font-size="26" font-weight="600" text-anchor="middle">${text}</text>
  <text x="50%" y="54%" fill="rgba(255,255,255,0.7)" font-family="system-ui,sans-serif" font-size="15" text-anchor="middle">placeholder · set REPLICATE_API_TOKEN for real images</text>
</svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}
