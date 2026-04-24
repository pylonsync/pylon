"use client";

import * as React from "react";
import Link from "next/link";
import { PylonMark } from "./pylon-logo";

const SECTION_LINKS = [
  { href: "#demo", label: "Demo" },
  { href: "#features", label: "Features" },
  { href: "#scale", label: "Scale" },
  { href: "#compare", label: "Compare" },
  { href: "#quickstart", label: "Quickstart" },
];

export function Nav() {
  const [open, setOpen] = React.useState(false);

  // Collapse the menu when any link is tapped, or when the viewport
  // grows past the mobile breakpoint (so the hamburger state doesn't
  // linger into the desktop layout after a resize).
  const close = React.useCallback(() => setOpen(false), []);
  React.useEffect(() => {
    const onResize = () => {
      if (window.innerWidth > 640) setOpen(false);
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return (
    <nav className="nav">
      <div className="container-page nav-inner">
        <div className="nav-left">
          <Link href="/" className="logo">
            <PylonMark size={22} />
            <span>Pylon</span>
          </Link>
          <div className="nav-links">
            {SECTION_LINKS.map((l) => (
              <a key={l.href} href={l.href}>
                {l.label}
              </a>
            ))}
          </div>
        </div>
        <div className="nav-right">
          <a
            href="https://docs.pylonsync.com"
            className="text-mono text-dim"
            style={{ fontSize: 12, marginRight: 6 }}
          >
            docs
          </a>
          <a
            className="inline-flex items-center gap-2 h-[30px] px-[11px] text-[12.5px] rounded-[5px] border border-[color:var(--border-2)] text-[color:var(--text)] hover:bg-[color:var(--bg-2)] hover:border-[#33333a] transition-colors font-medium"
            href="https://github.com/pylonsync/pylon"
            target="_blank"
            rel="noopener noreferrer"
          >
            <svg
              viewBox="0 0 24 24"
              fill="currentColor"
              style={{ width: 13, height: 13 }}
            >
              <path d="M12 .5C5.65.5.5 5.65.5 12a11.5 11.5 0 0 0 7.86 10.92c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.05-.72.08-.7.08-.7 1.16.08 1.77 1.2 1.77 1.2 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.24-1.28-5.24-5.7 0-1.26.45-2.3 1.19-3.11-.12-.3-.52-1.49.12-3.1 0 0 .97-.31 3.18 1.18a11 11 0 0 1 5.78 0c2.21-1.49 3.18-1.18 3.18-1.18.64 1.61.24 2.8.12 3.1.74.81 1.19 1.85 1.19 3.11 0 4.43-2.7 5.41-5.27 5.69.41.36.78 1.06.78 2.15v3.19c0 .31.21.67.8.56A11.5 11.5 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5z" />
            </svg>
            GitHub
          </a>
          <button
            className="nav-mobile-toggle"
            aria-label={open ? "Close menu" : "Open menu"}
            aria-expanded={open}
            onClick={() => setOpen((v) => !v)}
          >
            <svg
              viewBox="0 0 24 24"
              width="16"
              height="16"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden
            >
              {open ? (
                <>
                  <line x1="18" y1="6" x2="6" y2="18" />
                  <line x1="6" y1="6" x2="18" y2="18" />
                </>
              ) : (
                <>
                  <line x1="3" y1="6" x2="21" y2="6" />
                  <line x1="3" y1="12" x2="21" y2="12" />
                  <line x1="3" y1="18" x2="21" y2="18" />
                </>
              )}
            </svg>
          </button>
        </div>
      </div>
      <div className={`nav-mobile-menu ${open ? "open" : ""}`}>
        {SECTION_LINKS.map((l) => (
          <a key={l.href} href={l.href} onClick={close}>
            {l.label}
          </a>
        ))}
        <a href="https://docs.pylonsync.com" onClick={close}>
          Docs
        </a>
        <a
          href="https://github.com/pylonsync/pylon"
          target="_blank"
          rel="noopener noreferrer"
          onClick={close}
        >
          GitHub
        </a>
      </div>
    </nav>
  );
}
