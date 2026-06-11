import React from "react";
import {
  Armchair,
  Bike,
  Camera,
  CookingPot,
  Guitar,
  Headphones,
  MonitorSmartphone,
  Package,
  Shirt,
  Tent,
  type LucideIcon,
} from "lucide-react";

// Category → icon. Drives the listing "photo" placeholder, so the grid +
// detail pages read as a real catalog instead of text initials. Pure SVG
// (no client hooks), so it renders straight through SSR.
const ICONS: Record<string, LucideIcon> = {
  furniture: Armchair,
  electronics: MonitorSmartphone,
  cameras: Camera,
  bikes: Bike,
  audio: Headphones,
  kitchen: CookingPot,
  instruments: Guitar,
  outdoor: Tent,
  apparel: Shirt,
};

export function CategoryIcon({
  category,
  className,
}: {
  category: string;
  className?: string;
}) {
  const Icon = ICONS[category] ?? Package;
  return <Icon className={className} strokeWidth={1.25} aria-hidden />;
}
