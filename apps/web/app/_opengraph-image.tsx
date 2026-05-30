import { ImageResponse } from "next/og";

export const runtime = "edge";
export const alt = "Pylon — The realtime backend, finally finished.";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

// Fetches a single Google Font binary for Satori. Uses a legacy IE9
// User-Agent so the css2 endpoint returns a single un-subsetted woff
// file — the default modern-Chrome UA gets multiple subsetted woff2
// files (cyrillic / latin-ext / latin) that lack the tables Satori
// needs to render correctly. Multiple @font-face blocks may still come
// back; the latin block is last, so we take that one.
async function loadGoogleFont(
  family: string,
  weight: number,
  italic = false,
): Promise<ArrayBuffer> {
  const axis = italic ? `ital,wght@1,${weight}` : `wght@${weight}`;
  const url = `https://fonts.googleapis.com/css2?family=${family.replace(
    / /g,
    "+",
  )}:${axis}&display=swap`;
  const css = await fetch(url, {
    headers: {
      "User-Agent":
        "Mozilla/5.0 (compatible; MSIE 9.0; Windows NT 6.1; Trident/5.0)",
    },
  }).then((r) => r.text());
  const matches = [...css.matchAll(/src: url\((https:[^)]+)\) format/g)];
  const fontUrl = matches[matches.length - 1]?.[1];
  if (!fontUrl) throw new Error(`Could not extract font URL for ${family}`);
  return fetch(fontUrl).then((r) => r.arrayBuffer());
}

export default async function OpengraphImage() {
  const [geist, geistBold, instrumentSerifItalic] = await Promise.all([
    loadGoogleFont("Geist", 500),
    loadGoogleFont("Geist", 600),
    loadGoogleFont("Instrument Serif", 400, true),
  ]);

  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          background: "#fafaf9",
          padding: "72px 80px",
          fontFamily: "Geist",
          position: "relative",
        }}
      >
        {/* Graph-paper grid — Satori doesn't render mask-image, so we
            soften the grid with low opacity instead of a radial fade. */}
        <div
          style={{
            position: "absolute",
            inset: 0,
            backgroundImage:
              "linear-gradient(#e7e5e2 1px, transparent 1px), linear-gradient(90deg, #e7e5e2 1px, transparent 1px)",
            backgroundSize: "56px 56px",
            display: "flex",
            opacity: 0.4,
          }}
        />

        {/* Warm orange wash, top-right corner */}
        <div
          style={{
            position: "absolute",
            top: -160,
            right: -160,
            width: 600,
            height: 600,
            borderRadius: 300,
            background:
              "radial-gradient(circle, rgba(255,91,31,0.18) 0%, rgba(255,91,31,0) 65%)",
            display: "flex",
          }}
        />

        {/* Logo + wordmark */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 18,
          }}
        >
          <svg
            width="44"
            height="59"
            viewBox="0 0 48 64"
            fill="#0a0a0b"
            xmlns="http://www.w3.org/2000/svg"
          >
            <path d="M24 2 L10 20 L24 32 Z" />
            <path d="M24 2 L38 20 L24 32 Z" />
            <path d="M24 32 L18 48 L24 62 L30 48 Z" />
            <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
            <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
          </svg>
          <div
            style={{
              fontSize: 44,
              fontWeight: 600,
              color: "#0a0a0b",
              letterSpacing: "-0.02em",
            }}
          >
            Pylon
          </div>
        </div>

        {/* Headline */}
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            marginTop: "auto",
          }}
        >
          <div
            style={{
              display: "flex",
              fontSize: 92,
              fontWeight: 600,
              color: "#0a0a0b",
              letterSpacing: "-0.045em",
              lineHeight: 1,
            }}
          >
            The realtime backend,
          </div>
          <div
            style={{
              display: "flex",
              fontSize: 92,
              fontFamily: "Instrument Serif",
              fontStyle: "italic",
              fontWeight: 400,
              color: "#52525b",
              letterSpacing: "-0.02em",
              lineHeight: 1,
              marginTop: 6,
            }}
          >
            finally finished.
          </div>
          <div
            style={{
              display: "flex",
              fontSize: 28,
              color: "#52525b",
              fontWeight: 500,
              letterSpacing: "-0.005em",
              maxWidth: 880,
              lineHeight: 1.4,
              marginTop: 28,
            }}
          >
            Schema, server functions, live queries, auth, jobs, files, and
            search — in one binary.
          </div>
        </div>

        {/* Bottom row: command pill + URL */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginTop: 56,
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              padding: "10px 18px",
              border: "1px solid #d4d4d0",
              borderRadius: 10,
              background: "#ffffff",
              fontSize: 20,
              color: "#18181b",
            }}
          >
            <div style={{ display: "flex", color: "#a1a1aa" }}>$</div>
            <div style={{ display: "flex" }}>
              npm create @pylonsync/pylon@latest
            </div>
          </div>

          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              fontSize: 20,
              color: "#a1a1aa",
            }}
          >
            <div
              style={{
                display: "flex",
                width: 8,
                height: 8,
                borderRadius: 4,
                background: "#16a34a",
              }}
            />
            <div style={{ display: "flex" }}>pylonsync.com</div>
          </div>
        </div>
      </div>
    ),
    {
      ...size,
      fonts: [
        { name: "Geist", data: geist, style: "normal", weight: 500 },
        { name: "Geist", data: geistBold, style: "normal", weight: 600 },
        {
          name: "Instrument Serif",
          data: instrumentSerifItalic,
          style: "italic",
          weight: 400,
        },
      ],
    },
  );
}
