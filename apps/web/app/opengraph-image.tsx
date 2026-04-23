import { ImageResponse } from "next/og";

export const runtime = "edge";
export const alt = "Pylon — The realtime backend framework";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

export default async function OpengraphImage() {
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          background: "#0A0A0C",
          position: "relative",
          padding: "72px 80px",
          fontFamily: "system-ui, -apple-system, sans-serif",
        }}
      >
        {/* Subtle radial violet wash, top-right */}
        <div
          style={{
            position: "absolute",
            top: -200,
            right: -200,
            width: 700,
            height: 700,
            borderRadius: "50%",
            background:
              "radial-gradient(circle, rgba(139,92,246,0.22) 0%, rgba(139,92,246,0) 65%)",
            display: "flex",
          }}
        />

        {/* Grid pattern */}
        <div
          style={{
            position: "absolute",
            inset: 0,
            backgroundImage:
              "linear-gradient(rgba(255,255,255,0.03) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,0.03) 1px, transparent 1px)",
            backgroundSize: "56px 56px",
            display: "flex",
          }}
        />

        {/* Logo + wordmark */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 20,
            zIndex: 1,
          }}
        >
          <svg
            width="56"
            height="75"
            viewBox="0 0 48 64"
            fill="#8B5CF6"
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
              fontSize: 52,
              fontWeight: 500,
              color: "#F8FAFC",
              letterSpacing: "-0.02em",
              display: "flex",
            }}
          >
            Pylon
          </div>
        </div>

        {/* Main headline */}
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            marginTop: "auto",
            gap: 24,
            zIndex: 1,
          }}
        >
          <div
            style={{
              fontSize: 92,
              fontWeight: 300,
              color: "#F8FAFC",
              letterSpacing: "-0.035em",
              lineHeight: 1.02,
              maxWidth: 900,
              display: "flex",
              flexDirection: "column",
            }}
          >
            <span>The realtime</span>
            <span>
              backend{" "}
              <span style={{ color: "#A78BFA", fontStyle: "italic" }}>
                framework
              </span>
              .
            </span>
          </div>

          <div
            style={{
              fontSize: 28,
              color: "#9CA3AF",
              fontWeight: 400,
              letterSpacing: "-0.005em",
              maxWidth: 820,
              display: "flex",
            }}
          >
            Schema, policies, functions, live queries, auth — one binary.
          </div>
        </div>

        {/* Bottom row: pill + URL */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginTop: 56,
            zIndex: 1,
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 12,
              padding: "10px 18px",
              border: "1px solid #262626",
              borderRadius: 999,
              background: "rgba(20,20,20,0.6)",
              fontSize: 18,
              color: "#D4D4D8",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
            }}
          >
            <div
              style={{
                width: 8,
                height: 8,
                borderRadius: "50%",
                background: "#8B5CF6",
                boxShadow: "0 0 12px #8B5CF6",
                display: "flex",
              }}
            />
            <span style={{ display: "flex" }}>$ pylon dev</span>
          </div>

          <div
            style={{
              fontSize: 22,
              color: "#6B7280",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
              display: "flex",
            }}
          >
            pylonsync.com
          </div>
        </div>
      </div>
    ),
    { ...size },
  );
}
