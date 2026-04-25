"use client";

import { useRouter, useSearchParams } from "next/navigation";

const OPTIONS = [
  { value: "relevance", label: "Relevance" },
  { value: "price-asc", label: "Price: low to high" },
  { value: "price-desc", label: "Price: high to low" },
  { value: "rating-desc", label: "Highest rated" },
  { value: "newest", label: "Newest" },
];

export function SortSelect({ value }: { value: string }) {
  const router = useRouter();
  const params = useSearchParams();

  const onChange = (next: string) => {
    const sp = new URLSearchParams(params.toString());
    if (next === "relevance") sp.delete("sort");
    else sp.set("sort", next);
    sp.delete("page");
    router.replace(`/?${sp.toString()}`, { scroll: false });
  };

  return (
    <label className="flex items-center gap-2 text-xs text-muted-foreground">
      Sort
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-8 rounded-md border bg-background px-2 text-xs text-foreground"
      >
        {OPTIONS.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}
