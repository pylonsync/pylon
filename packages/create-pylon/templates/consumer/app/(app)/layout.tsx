import React from "react";
import { AppChrome } from "@/components/social/app-chrome";

interface LayoutProps {
  children: React.ReactNode;
}

// `(app)` is a route group: the parens segment is not part of any URL. This
// layout adds the navigation around every page of the app.
export default function AppLayout({ children }: LayoutProps) {
  return (
    <>
      <AppChrome />
      <main className="pb-14 md:pb-0 md:pl-[72px] xl:pl-[244px]">{children}</main>
    </>
  );
}
