// THE single source of truth for everything brand-specific on this AI media
// studio. Rebrand by editing this ONE file — the layout + studio UI read from
// here. The create-pylon scaffolder and Mast target this file.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
// Fictional demo copy — replace the values, keep the shape.

import type { GenerationKind } from "./studio";

export type Social = { label: string; href: string; path: string };

export type BaseConfig = {
  brand: {
    name: string;
    letter: string;
    domain: string;
    email: string;
    footerBlurb: string;
    copyrightName: string;
    socials: Social[];
  };
  colors: { brand: string; brandSoft: string; paper: string };
  seo: { title: string; description: string };
};

export type StudioConfig = BaseConfig & {
  studio: {
    headline: string;
    subcopy: string;
    inputPlaceholder: string;
    // The generation kinds shown as a selector. `wired` flags which call a
    // provider (all three do via Replicate when REPLICATE_API_TOKEN is set).
    kinds: { id: GenerationKind; label: string; wired: boolean }[];
    // Starter prompts; clicking one fills the box.
    examples: string[];
  };
};

export const siteConfig: StudioConfig = {
  brand: {
    name: "Prism",
    letter: "P",
    domain: "prism.studio",
    email: "hello@prism.example",
    footerBlurb: "A generative media studio built on Pylon — image, audio, and video from a prompt.",
    copyrightName: "Prism",
    socials: [
      {
        label: "X",
        href: "https://x.com",
        path: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
      },
    ],
  },

  colors: { brand: "#c026d3", brandSoft: "#fae8ff", paper: "#fdf9fe" },

  seo: {
    title: "Prism — a generative AI media studio.",
    description:
      "Generate images, audio, and video from a prompt. A live gallery that updates in realtime as each generation finishes, built on Pylon. Your API key stays server-side.",
  },

  studio: {
    headline: "Make something.",
    subcopy: "Describe it, pick a medium, and watch it land in your gallery — live, the moment it's done.",
    inputPlaceholder: "A neon koi swimming through a rainy Tokyo alley at night…",
    kinds: [
      { id: "image", label: "Image", wired: true },
      { id: "audio", label: "Audio", wired: true },
      { id: "video", label: "Video", wired: true },
    ],
    examples: [
      "A cozy reading nook in a treehouse, golden hour, watercolor",
      "An upbeat lo-fi hip-hop loop with mellow piano",
      "Isometric illustration of a tiny island data center",
      "A timelapse of a city skyline as day turns to night",
    ],
  },
};
