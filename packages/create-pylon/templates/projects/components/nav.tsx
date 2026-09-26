import React from "react";
import { FolderKanban, Users } from "lucide-react";

export interface NavEntry {
  href: string;
  label: string;
  icon: React.ReactNode;
}

/** The sidebar's links, top to bottom. Add a view here to put it in the nav. */
export const NAV: NavEntry[] = [
  { href: "/", label: "Projects", icon: <FolderKanban /> },
  { href: "/clients", label: "Clients", icon: <Users /> },
];
