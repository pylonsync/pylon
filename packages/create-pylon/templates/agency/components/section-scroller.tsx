"use client";

import { useEffect } from "react";

// Makes in-page section links work. A hydrated Pylon page updates the URL for a
// plain `<a href="#section">` click but doesn't perform the browser's native
// fragment scroll, so the page jumps nowhere. This installs ONE delegated click
// handler that catches any same-page `#`/`/#` anchor and scrolls to it smoothly.
//
// Render it once (in the root layout). Renders nothing. Real route links should
// still use `<Link>` from @pylonsync/react — this only handles `#` anchors.
export function SectionScroller() {
  useEffect(() => {
    function onClick(e: MouseEvent) {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) {
        return;
      }
      const target = e.target as Element | null;
      const link = target?.closest?.('a[href^="#"], a[href^="/#"]') as HTMLAnchorElement | null;
      if (!link) return;
      const href = link.getAttribute("href") || "";
      const id = href.slice(href.indexOf("#") + 1);
      if (!id) return;
      const el = document.getElementById(id);
      if (!el) return; // target not on this page — leave it to the browser
      e.preventDefault();
      el.scrollIntoView({ block: "start" });
      history.replaceState(null, "", "#" + id);
    }
    document.addEventListener("click", onClick);
    return () => document.removeEventListener("click", onClick);
  }, []);

  return null;
}
