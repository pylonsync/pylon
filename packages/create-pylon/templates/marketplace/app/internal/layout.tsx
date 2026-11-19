import React from "react";
import { Link, type Metadata } from "@pylonsync/react";
import {
  COMPONENT_SECTIONS,
  DESIGN_SECTIONS,
  type InternalSection,
} from "./sections";

// `/internal/*` is the reference for the design tokens and the components of
// this app. The product does not link to these pages and search engines do
// not index them. To restrict them to signed-in staff, read the `auth` prop
// here and call `response.notFound()`.
export const metadata: Metadata = {
  title: "Internal",
  robots: "noindex, nofollow",
};

interface InternalLayoutProps {
  children: React.ReactNode;
  pathname: string;
}

interface NavGroupProps {
  title: string;
  href: string;
  sections: InternalSection[];
  current: boolean;
}

function NavGroup({ title, href, sections, current }: NavGroupProps) {
  return (
    <div className="flex flex-col gap-1">
      <Link
        href={href}
        aria-current={current ? "page" : undefined}
        className="rounded-md px-2 py-1.5 text-sm font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground aria-[current=page]:bg-accent aria-[current=page]:text-accent-foreground"
      >
        {title}
      </Link>
      <ul className="ml-2 hidden flex-col border-l pl-2 md:flex">
        {sections.map((section) => (
          <li key={section.id}>
            <a
              href={`${href}#${section.id}`}
              className="block rounded-md px-2 py-1 text-sm text-muted-foreground hover:text-foreground"
            >
              {section.title}
            </a>
          </li>
        ))}
      </ul>
    </div>
  );
}

export default function InternalLayout({ children, pathname }: InternalLayoutProps) {
  return (
    <div className="flex min-h-screen flex-col bg-background text-foreground md:flex-row">
      <aside className="border-b md:sticky md:top-0 md:h-screen md:w-60 md:shrink-0 md:overflow-y-auto md:border-b-0 md:border-r">
        <nav
          aria-label="Internal"
          className="flex flex-row items-start gap-2 p-3 md:flex-col md:gap-4 md:p-4"
        >
          <Link
            href="/"
            className="px-2 py-1.5 text-sm font-semibold tracking-tight"
          >
            __APP_NAME__
          </Link>
          <NavGroup
            title="Design"
            href="/internal/design"
            sections={DESIGN_SECTIONS}
            current={pathname === "/internal/design"}
          />
          <NavGroup
            title="Components"
            href="/internal/components"
            sections={COMPONENT_SECTIONS}
            current={pathname === "/internal/components"}
          />
        </nav>
      </aside>
      <main className="min-w-0 flex-1 px-6 py-10 md:px-10">
        <div className="mx-auto flex max-w-3xl flex-col">{children}</div>
      </main>
    </div>
  );
}
