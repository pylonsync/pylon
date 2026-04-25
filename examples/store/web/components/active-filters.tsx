import Link from "next/link";
import { X } from "lucide-react";
import { Badge } from "@pylonsync/example-ui/badge";

const FACET_LABELS: Record<string, string> = {
  brand: "Brand",
  category: "Category",
  color: "Color",
};

export function ActiveFilters({
  filters,
  query,
}: {
  filters: Record<string, string>;
  query: string;
}) {
  const entries = Object.entries(filters);
  if (entries.length === 0 && !query) return <span />;
  return (
    <div className="flex flex-wrap gap-2">
      {query && (
        <Badge variant="secondary" className="capitalize">
          “{query}”
        </Badge>
      )}
      {entries.map(([facet, value]) => {
        const params = new URLSearchParams();
        for (const [k, v] of entries) {
          if (k !== facet) params.set(k, v);
        }
        if (query) params.set("q", query);
        const href = params.toString() ? `/?${params.toString()}` : "/";
        return (
          <Link
            key={`${facet}:${value}`}
            href={href}
            scroll={false}
          >
            <Badge variant="secondary" className="cursor-pointer gap-1 capitalize">
              {FACET_LABELS[facet] ?? facet}: {value}
              <X className="size-3" />
            </Badge>
          </Link>
        );
      })}
    </div>
  );
}
