import React from "react";
import { ArrowLeftRight, Boxes } from "lucide-react";

export interface NavEntry {
  href: string;
  label: string;
  icon: React.ReactNode;
}

/** The sidebar's links, top to bottom. Add a view here to put it in the nav. */
export const NAV: NavEntry[] = [
  { href: "/", label: "Products", icon: <Boxes /> },
  { href: "/movements", label: "Movements", icon: <ArrowLeftRight /> },
];
