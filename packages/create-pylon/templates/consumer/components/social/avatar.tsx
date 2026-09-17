"use client";

import React from "react";
import { cn } from "@/lib/utils";
import type { Profile } from "@/lib/social";

/** A round profile photo, or the first letter of the name when there is none. */
export function Avatar({
  profile,
  size = 32,
  ring = false,
  className,
}: {
  profile: Pick<Profile, "username" | "displayName" | "avatarUrl"> | null | undefined;
  size?: number;
  /** A colored ring, for people who posted recently. */
  ring?: boolean;
  className?: string;
}) {
  const letter = (profile?.displayName || profile?.username || "?").trim().charAt(0).toUpperCase();
  const inner = profile?.avatarUrl ? (
    <img
      src={profile.avatarUrl}
      alt=""
      width={size}
      height={size}
      loading="lazy"
      className="size-full rounded-full object-cover"
    />
  ) : (
    <span
      className="grid size-full place-items-center rounded-full bg-zinc-200 font-semibold text-zinc-600"
      style={{ fontSize: Math.max(10, Math.round(size * 0.4)) }}
    >
      {letter}
    </span>
  );
  return (
    <span
      className={cn(
        "inline-block shrink-0 rounded-full",
        ring ? "bg-brand p-[2px]" : "",
        className,
      )}
      style={{ width: size + (ring ? 4 : 0), height: size + (ring ? 4 : 0) }}
    >
      <span className={cn("block size-full rounded-full", ring ? "bg-background p-[2px]" : "")}>{inner}</span>
    </span>
  );
}
