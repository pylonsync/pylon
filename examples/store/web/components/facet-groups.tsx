/**
 * Server-rendered facet sidebar.
 *
 * Each facet value links to a URL with that facet selected (or removed,
 * if it's already active). Crawlers see real `<a href="?brand=...">`
 * links and can crawl the facet space — useful for SEO of category /
 * brand landing pages.
 */
import Link from "next/link";

const FACET_LABELS: Record<string, string> = {
  brand: "Brand",
  category: "Category",
  color: "Color",
};
const FACET_ORDER = ["category", "brand", "color"];

export function FacetGroups({
  facetCounts,
  active,
}: {
  facetCounts: Record<string, Record<string, number>>;
  active: Record<string, string>;
}) {
  return (
    <div className="flex flex-col gap-5">
      {FACET_ORDER.map((facet) => {
        const counts = facetCounts[facet];
        if (!counts) return null;
        const entries = Object.entries(counts).sort((a, b) => b[1] - a[1]);
        return (
          <div key={facet} className="flex flex-col gap-1.5">
            <h4 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              {FACET_LABELS[facet] ?? facet}
            </h4>
            <ul className="flex flex-col gap-0.5">
              {entries.slice(0, 10).map(([value, count]) => {
                const on = active[facet] === value;
                const href = facetHref(facet, value, active, on);
                return (
                  <li key={value}>
                    <Link
                      href={href}
                      scroll={false}
                      className={
                        "flex w-full items-center justify-between rounded-md px-2 py-1.5 text-sm capitalize transition-colors " +
                        (on
                          ? "bg-primary text-primary-foreground"
                          : "text-foreground/80 hover:bg-accent hover:text-accent-foreground")
                      }
                    >
                      <span>{value}</span>
                      <span
                        className={
                          "font-mono text-[11px] " +
                          (on ? "opacity-80" : "text-muted-foreground")
                        }
                      >
                        {count}
                      </span>
                    </Link>
                  </li>
                );
              })}
            </ul>
          </div>
        );
      })}
    </div>
  );
}

function facetHref(
  facet: string,
  value: string,
  active: Record<string, string>,
  isOn: boolean,
): string {
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(active)) {
    if (k === facet) continue;
    params.set(k, v);
  }
  if (!isOn) params.set(facet, value);
  // Drop the page param so a new filter resets to page 1.
  params.delete("page");
  return params.toString() ? `/?${params.toString()}` : "/";
}
