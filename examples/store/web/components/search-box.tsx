"use client";

/**
 * Search input that drives the URL `?q=` state.
 *
 * Debounces by 250ms so a fast typer doesn't trigger a Next route
 * change per keystroke. Uses `router.replace` so the back button
 * doesn't fill up with intermediate searches.
 */
import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { Input } from "@pylonsync/example-ui/input";

export function SearchBox({ initialValue }: { initialValue: string }) {
  const router = useRouter();
  const [value, setValue] = useState(initialValue);

  useEffect(() => {
    setValue(initialValue);
  }, [initialValue]);

  useEffect(() => {
    const trimmed = value.trim();
    if (trimmed === initialValue) return;
    const id = setTimeout(() => {
      const params = new URLSearchParams(window.location.search);
      if (trimmed) params.set("q", trimmed);
      else params.delete("q");
      params.delete("page");
      router.replace(`/?${params.toString()}`, { scroll: false });
    }, 250);
    return () => clearTimeout(id);
  }, [value, initialValue, router]);

  return (
    <Input
      value={value}
      onChange={(e) => setValue(e.target.value)}
      placeholder="Search 10,000 products…"
      className="h-10 max-w-xl"
      aria-label="Search products"
    />
  );
}
