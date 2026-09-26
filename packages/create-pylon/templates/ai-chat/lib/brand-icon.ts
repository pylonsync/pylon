import { siteConfig } from "./site.config";

// The brand mark as an SVG data URL, used as the browser tab icon. Same
// shapes as BrandMark in components/chat/icons.tsx.
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="9" fill="${siteConfig.colors.brand}"/><circle cx="16" cy="16" r="8.5" fill="none" stroke="#fff" stroke-opacity="0.35" stroke-width="1.5"/><circle cx="13.5" cy="13.5" r="4" fill="#fff"/></svg>`;

export const brandIcon = { url: `data:image/svg+xml,${encodeURIComponent(svg)}`, type: "image/svg+xml" };
