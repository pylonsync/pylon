export function hashColor(s: string): string {
  let h = 0;
  for (let i = 0; i < s.length; i++) {
    h = (h * 31 + s.charCodeAt(i)) | 0;
  }
  const hue = Math.abs(h) % 360;
  return `hsl(${hue}, 50%, 55%)`;
}

export function gradient(a: string, b: string) {
  return `linear-gradient(135deg, ${hashColor(a)}, ${hashColor(b)})`;
}

export function initials(name: string) {
  return name
    .split(" ")
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .join("");
}

export function formatPrice(n: number) {
  return `$${n.toFixed(2)}`;
}

export function navigate(href: string) {
  if (window.location.hash === href) return;
  window.location.hash = href;
}
