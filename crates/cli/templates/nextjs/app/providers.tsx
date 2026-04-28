"use client";

import { useEffect } from "react";
import { configureClient } from "@pylonsync/react";

// Configure the client-side Pylon SDK. baseUrl is empty so requests
// go same-origin and the Next.js proxy in next.config.js forwards
// them to PYLON_TARGET. The session cookie rides along automatically.
export function Providers({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    configureClient({ baseUrl: "" });
  }, []);
  return <>{children}</>;
}
