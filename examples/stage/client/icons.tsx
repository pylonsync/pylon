/**
 * Icon wrapper — renders real SVG icons from Lucide.
 *
 * The Site tokens carry an `iconLibrary` knob matching shadcn/ui's
 * create page (Lucide / Tabler / HugeIcons / Phosphor / Remix), but
 * the runtime currently resolves every name through Lucide to keep
 * bundle size sane. Other libraries can be wired in later without
 * touching callers — just import and branch in `Icon` below.
 */
import React from "react";
import * as Lucide from "lucide-react";

export type IconLibrary = "lucide" | "tabler" | "hugeicons" | "phosphor" | "remix";

export const ICON_LIBRARIES: { id: IconLibrary; label: string }[] = [
  { id: "lucide", label: "Lucide" },
  { id: "tabler", label: "Tabler Icons" },
  { id: "hugeicons", label: "HugeIcons" },
  { id: "phosphor", label: "Phosphor Icons" },
  { id: "remix", label: "Remix Icon" },
];

const LibraryContext = React.createContext<IconLibrary>("lucide");

export function IconLibraryProvider({
  library, children,
}: { library: IconLibrary; children: React.ReactNode }) {
  return <LibraryContext.Provider value={library}>{children}</LibraryContext.Provider>;
}

export function Icon({
  name, size = 16, color, className, strokeWidth,
}: {
  name: string;
  size?: number | string;
  color?: string;
  className?: string;
  strokeWidth?: number;
  library?: IconLibrary;
}) {
  const Component = (Lucide as any)[name] as
    | React.ComponentType<{ size?: number | string; color?: string; className?: string; strokeWidth?: number }>
    | undefined;
  if (!Component) {
    // Unknown name — render a neutral placeholder rather than crash.
    return (
      <span
        className={className}
        style={{
          display: "inline-block",
          width: size, height: size,
          border: "1px dashed currentColor",
          borderRadius: 3,
          opacity: 0.4,
        }}
        aria-hidden
      />
    );
  }
  return <Component size={size} color={color} className={className} strokeWidth={strokeWidth} />;
}
