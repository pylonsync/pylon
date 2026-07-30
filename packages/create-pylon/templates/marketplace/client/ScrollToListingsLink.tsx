"use client";

import React from "react";
import { Button } from "@/components/ui/button";

export function ScrollToListingsLink({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <Button asChild className={className}>
      <a
        href="#listings"
        onClick={(event) => {
          const listings = document.getElementById("listings");
          if (!listings) return;
          event.preventDefault();
          history.replaceState(null, "", "#listings");
          listings.scrollIntoView({ behavior: "smooth", block: "start" });
        }}
      >
        {children}
      </a>
    </Button>
  );
}
