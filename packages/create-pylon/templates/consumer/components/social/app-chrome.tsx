"use client";

import React, { useState } from "react";
import { Link, usePathname } from "@pylonsync/react";
import { Compass, House, SquarePlus } from "lucide-react";
import { SITE } from "@/lib/site";
import { cn } from "@/lib/utils";
import { Avatar } from "./avatar";
import { CreatePostDialog } from "./create-post";
import { SocialGate } from "./session";
import { useSocial } from "./use-social";

/**
 * Navigation: a sidebar from the md breakpoint up, and a top bar with a tab
 * bar underneath the page below it. Create opens the new-post dialog.
 */
export function AppChrome() {
  return (
    <SocialGate fallback={<ChromeFrame profileHref="/" profile={null} onCreate={() => {}} />}>
      <LiveChrome />
    </SocialGate>
  );
}

function LiveChrome() {
  const social = useSocial();
  const [creating, setCreating] = useState(false);
  const href = social.myProfile ? `/u/${social.myProfile.username}` : "/";
  return (
    <>
      <ChromeFrame profileHref={href} profile={social.myProfile} onCreate={() => setCreating(true)} />
      <CreatePostDialog open={creating} onOpenChange={setCreating} />
    </>
  );
}

function ChromeFrame({
  profileHref,
  profile,
  onCreate,
}: {
  profileHref: string;
  profile: Parameters<typeof Avatar>[0]["profile"];
  onCreate: () => void;
}) {
  const path = usePathname();
  const onProfile = profileHref !== "/" && path === profileHref;
  const items = [
    { label: "Home", href: "/", active: path === "/", icon: House },
    { label: "Explore", href: "/explore", active: path === "/explore", icon: Compass },
  ];

  return (
    <>
      {/* Sidebar, md and up. */}
      <nav className="fixed inset-y-0 left-0 z-30 hidden w-[72px] flex-col border-r bg-background px-3 pb-5 pt-7 md:flex xl:w-[244px]">
        <Link href="/" className="mb-8 flex h-10 items-center px-3">
          <span className="font-display text-[28px] italic leading-none xl:inline hidden">{SITE.name}</span>
          <span className="font-display text-[28px] italic leading-none xl:hidden">{SITE.name.charAt(0)}</span>
        </Link>
        <div className="flex flex-col gap-1">
          {items.map((item) => (
            <Link
              key={item.href}
              href={item.href}
              className="flex h-12 items-center gap-4 rounded-lg px-3 transition-colors hover:bg-zinc-100"
            >
              <item.icon className="size-6 shrink-0" strokeWidth={item.active ? 2.5 : 1.75} />
              <span className={cn("hidden text-[15px] xl:inline", item.active ? "font-bold" : "")}>{item.label}</span>
            </Link>
          ))}
          <button type="button" onClick={onCreate} className="flex h-12 items-center gap-4 rounded-lg px-3 text-left transition-colors hover:bg-zinc-100">
            <SquarePlus className="size-6 shrink-0" strokeWidth={1.75} />
            <span className="hidden text-[15px] xl:inline">Create</span>
          </button>
          <Link href={profileHref} className="flex h-12 items-center gap-4 rounded-lg px-3 transition-colors hover:bg-zinc-100">
            <Avatar profile={profile} size={24} className={onProfile ? "outline outline-2 outline-offset-1 outline-zinc-900" : ""} />
            <span className={cn("hidden text-[15px] xl:inline", onProfile ? "font-bold" : "")}>Profile</span>
          </Link>
        </div>
      </nav>

      {/* Top bar, below md. */}
      <header className="sticky top-0 z-30 flex h-12 items-center justify-between border-b bg-background px-4 md:hidden">
        <Link href="/" className="font-display text-[26px] italic leading-none">
          {SITE.name}
        </Link>
        <button type="button" onClick={onCreate} aria-label="Create" className="p-1">
          <SquarePlus className="size-6" strokeWidth={1.75} />
        </button>
      </header>

      {/* Tab bar, below md. */}
      <nav className="fixed inset-x-0 bottom-0 z-30 flex h-12 items-center justify-around border-t bg-background md:hidden">
        {items.map((item) => (
          <Link key={item.href} href={item.href} aria-label={item.label} className="p-2">
            <item.icon className="size-6" strokeWidth={item.active ? 2.5 : 1.75} />
          </Link>
        ))}
        <button type="button" onClick={onCreate} aria-label="Create" className="p-2">
          <SquarePlus className="size-6" strokeWidth={1.75} />
        </button>
        <Link href={profileHref} aria-label="Profile" className="p-2">
          <Avatar profile={profile} size={24} />
        </Link>
      </nav>
    </>
  );
}
