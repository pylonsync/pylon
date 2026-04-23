/**
 * shadcn/ui-style presets — a curated set that maps each shadcn theme
 * (Stone, Zinc, Red, Rose, Orange, Blue, Green, Slate) to our Site
 * tokens shape so users get "pick a preset, whole site restyles" in
 * one click.
 *
 * Also exposes a parser for shadcn's "Open Preset" paste format —
 * either a JSON blob (from ui.shadcn.com/create) or a block of CSS
 * custom properties using `hsl(H S% L%)` notation.
 */

export type PresetTokens = {
  colors: {
    primary: string; accent: string; body: string;
    muted: string; surface: string; outline: string;
  };
  rounded?: { md?: number; lg?: number };
  radius?: "none" | "sm" | "md" | "lg" | "xl";
  style?: string;
};

// Hand-tuned shadcn-inspired palettes. Values use hex for Site token
// compatibility (we don't carry hsl tuples) and were picked by
// eyeballing the shadcn create page for the listed style name.
export const SHADCN_PRESETS: { id: string; label: string; tokens: PresetTokens }[] = [
  {
    id: "neutral",
    label: "Neutral",
    tokens: {
      colors: {
        primary: "#0A0A0A", accent: "#171717", body: "#404040",
        muted: "#737373", surface: "#FFFFFF", outline: "#E5E5E5",
      },
      radius: "md",
    },
  },
  {
    id: "stone",
    label: "Stone",
    tokens: {
      colors: {
        primary: "#0C0A09", accent: "#292524", body: "#44403C",
        muted: "#78716C", surface: "#FAFAF9", outline: "#E7E5E4",
      },
      radius: "md",
    },
  },
  {
    id: "zinc",
    label: "Zinc",
    tokens: {
      colors: {
        primary: "#09090B", accent: "#18181B", body: "#3F3F46",
        muted: "#71717A", surface: "#FAFAFA", outline: "#E4E4E7",
      },
      radius: "md",
    },
  },
  {
    id: "red",
    label: "Red",
    tokens: {
      colors: {
        primary: "#0A0A0A", accent: "#DC2626", body: "#404040",
        muted: "#737373", surface: "#FFFFFF", outline: "#FECACA",
      },
      radius: "md",
    },
  },
  {
    id: "rose",
    label: "Rose",
    tokens: {
      colors: {
        primary: "#0A0A0A", accent: "#E11D48", body: "#404040",
        muted: "#737373", surface: "#FFFFFF", outline: "#FECDD3",
      },
      radius: "lg",
    },
  },
  {
    id: "orange",
    label: "Orange",
    tokens: {
      colors: {
        primary: "#0A0A0A", accent: "#F97316", body: "#404040",
        muted: "#737373", surface: "#FFFAF5", outline: "#FFEDD5",
      },
      radius: "md",
    },
  },
  {
    id: "blue",
    label: "Blue",
    tokens: {
      colors: {
        primary: "#0A0A0A", accent: "#2563EB", body: "#404040",
        muted: "#737373", surface: "#FFFFFF", outline: "#DBEAFE",
      },
      radius: "md",
    },
  },
  {
    id: "green",
    label: "Green",
    tokens: {
      colors: {
        primary: "#0A0A0A", accent: "#16A34A", body: "#404040",
        muted: "#737373", surface: "#FFFFFF", outline: "#D1FAE5",
      },
      radius: "md",
    },
  },
  {
    id: "violet",
    label: "Violet",
    tokens: {
      colors: {
        primary: "#0A0A0A", accent: "#7C3AED", body: "#404040",
        muted: "#737373", surface: "#FFFFFF", outline: "#EDE9FE",
      },
      radius: "lg",
    },
  },
  {
    id: "nova",
    label: "Nova",
    tokens: {
      colors: {
        primary: "#0B0B0F", accent: "#FF3D7F", body: "#334155",
        muted: "#64748B", surface: "#FFFFFF", outline: "#E6E4EB",
      },
      radius: "lg",
      style: "nova",
    },
  },
];

// Convert "220 14.3% 95.9%" → "#f1f2f4". Accepts variants shadcn uses.
function hslToHex(hslStr: string): string | null {
  const m = hslStr.trim().match(/^(\d+(?:\.\d+)?)\s+(\d+(?:\.\d+)?)%\s+(\d+(?:\.\d+)?)%$/);
  if (!m) return null;
  const h = Number(m[1]) / 360;
  const s = Number(m[2]) / 100;
  const l = Number(m[3]) / 100;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => {
    const k = (n + h * 12) % 12;
    const c = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(c * 255).toString(16).padStart(2, "0");
  };
  return `#${f(0)}${f(8)}${f(4)}`;
}

/**
 * Parse a shadcn theme blob into Site tokens. Accepts:
 *   - shadcn create-page JSON (keys: name, cssVars.theme.{background,foreground,primary,...})
 *   - Raw CSS block with `--primary: 240 5.9% 10%;` lines
 *   - Our own PresetTokens JSON (pass-through)
 */
export function parseShadcnPreset(input: string): PresetTokens | null {
  const trimmed = input.trim();
  if (!trimmed) return null;

  // Our own preset shape — pass through.
  try {
    const json = JSON.parse(trimmed);
    if (json?.tokens?.colors) return json.tokens as PresetTokens;
    // shadcn create JSON
    const theme = json?.cssVars?.theme ?? json?.theme ?? json?.cssVars ?? null;
    if (theme && typeof theme === "object") {
      return extractFromCssVars(theme);
    }
  } catch {
    // not JSON, fall through to CSS parsing
  }

  // Raw CSS — collect --var: value pairs.
  const vars: Record<string, string> = {};
  const varRe = /--([a-z0-9-]+)\s*:\s*([^;]+);/gi;
  let m: RegExpExecArray | null;
  while ((m = varRe.exec(trimmed)) !== null) {
    vars[m[1].toLowerCase()] = m[2].trim();
  }
  if (Object.keys(vars).length === 0) return null;
  return extractFromCssVars(vars);
}

function extractFromCssVars(vars: Record<string, string>): PresetTokens | null {
  const toHex = (v: string | undefined): string | null => {
    if (!v) return null;
    if (v.startsWith("#")) return v;
    return hslToHex(v);
  };
  const primary = toHex(vars["foreground"]) ?? toHex(vars["primary"]) ?? "#0A0A0A";
  const accent = toHex(vars["primary"]) ?? toHex(vars["accent"]) ?? "#000000";
  const body = toHex(vars["muted-foreground"]) ?? toHex(vars["foreground"]) ?? "#404040";
  const muted = toHex(vars["muted-foreground"]) ?? "#737373";
  const surface = toHex(vars["background"]) ?? toHex(vars["card"]) ?? "#FFFFFF";
  const outline = toHex(vars["border"]) ?? toHex(vars["input"]) ?? "#E5E5E5";

  const radiusRaw = vars["radius"];
  const radius: PresetTokens["radius"] | undefined = radiusRaw
    ? (parseFloat(radiusRaw) >= 1 ? "xl"
       : parseFloat(radiusRaw) >= 0.75 ? "lg"
       : parseFloat(radiusRaw) >= 0.5 ? "md"
       : parseFloat(radiusRaw) > 0 ? "sm" : "none")
    : undefined;

  return {
    colors: { primary, accent, body, muted, surface, outline },
    radius,
  };
}
