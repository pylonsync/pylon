"use client";

import React from "react";
import { cn } from "@/lib/utils";
import type { Profile } from "@/lib/social";

/** The same tints, in the same order, as the iOS app's AvatarView. */
const TINTS = ["#ff9500", "#ff2d55", "#af52de", "#5856d6", "#30b0c7", "#34c759", "#007aff", "#ff3b30"];

/** A stable tint per username. Same hash as the iOS app, so a person has one color everywhere. */
export function avatarTint(username: string): string {
  let h = 0;
  for (const ch of username) h = (Math.imul(h, 31) + (ch.codePointAt(0) ?? 0)) & 0xffff;
  return TINTS[h % TINTS.length];
}

/** A round profile photo, or the first letter of the name on a tinted circle. */
export function Avatar({
  profile,
  size = 32,
  ring = false,
  className,
}: {
  profile: Pick<Profile, "username" | "displayName" | "avatarUrl"> | null | undefined;
  size?: number;
  /** The gradient ring, for people who posted in the last 24 hours. */
  ring?: boolean;
  className?: string;
}) {
  const letter = (profile?.displayName || profile?.username || "?").trim().charAt(0).toUpperCase();
  const tint = avatarTint(profile?.username ?? "");
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
      aria-hidden
      className="grid size-full place-items-center rounded-full font-semibold text-white"
      style={{
        fontSize: Math.max(10, Math.round(size * 0.42)),
        background: `linear-gradient(to bottom, color-mix(in srgb, ${tint} 78%, white), ${tint})`,
      }}
    >
      {letter}
    </span>
  );
  const pad = ring ? Math.max(2, Math.round(size * 0.05)) : 0;
  return (
    <span
      className={cn("inline-block shrink-0 rounded-full", ring ? "bg-story-ring" : "", className)}
      style={{ width: size + pad * 4, height: size + pad * 4, padding: pad }}
    >
      <span className="block size-full rounded-full bg-background" style={{ padding: pad }}>
        {inner}
      </span>
    </span>
  );
}
