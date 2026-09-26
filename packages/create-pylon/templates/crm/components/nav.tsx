import React from "react";
import { Building2, KanbanSquare, Users } from "lucide-react";

export interface NavEntry {
  href: string;
  label: string;
  icon: React.ReactNode;
}

/** The sidebar's links, top to bottom. Add a view here to put it in the nav. */
export const NAV: NavEntry[] = [
  { href: "/", label: "Pipeline", icon: <KanbanSquare /> },
  { href: "/companies", label: "Companies", icon: <Building2 /> },
  { href: "/contacts", label: "Contacts", icon: <Users /> },
];
