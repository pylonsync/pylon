"use client";

import React, { useState } from "react";
import { Link } from "@pylonsync/react";
import { Bookmark, Grid3x3 } from "lucide-react";
import { formatCount } from "@/lib/social";
import { cn } from "@/lib/utils";
import { Avatar } from "./avatar";
import { EditProfileDialog } from "./edit-profile";
import { GridSkeleton, PostGrid } from "./post-grid";
import { SocialGate } from "./session";
import { actions, useSocial } from "./use-social";

export function ProfileScreen({ username }: { username: string }) {
  return (
    <div className="mx-auto max-w-[935px] pb-8 sm:px-5 sm:pt-8">
      <SocialGate fallback={<GridSkeleton />}>
        <LiveProfile username={username.toLowerCase()} />
      </SocialGate>
    </div>
  );
}

function LiveProfile({ username }: { username: string }) {
  const social = useSocial();
  const [tab, setTab] = useState<"posts" | "saved">("posts");
  const [editing, setEditing] = useState(false);
  const profile = social.profileByUsername.get(username);

  if (!profile) {
    return social.loading || social.profiles.length === 0 ? (
      <GridSkeleton />
    ) : (
      <div className="py-24 text-center">
        <p className="text-lg font-semibold">This profile is not available.</p>
        <p className="mt-2 text-sm text-zinc-500">The username may have changed.</p>
        <Link href="/" className="mt-4 inline-block text-sm font-semibold text-brand">
          Back to the feed
        </Link>
      </div>
    );
  }

  const mine = profile.userId === social.me;
  const posts = social.postsByAuthor.get(profile.userId) ?? [];
  const saved = mine ? social.posts.filter((p) => social.mySaves.has(p.id)) : [];
  const following = social.following.has(profile.userId);
  const followsYou = social.follows.some((f) => f.followerId === profile.userId && f.followingId === social.me);
  const stats = [
    { label: posts.length === 1 ? "post" : "posts", value: posts.length },
    { label: (social.followerCount.get(profile.userId) ?? 0) === 1 ? "follower" : "followers", value: social.followerCount.get(profile.userId) ?? 0 },
    { label: "following", value: social.followingCount.get(profile.userId) ?? 0 },
  ];

  const button = mine ? (
    <button type="button" onClick={() => setEditing(true)} className="rounded-lg bg-zinc-100 px-4 py-1.5 text-sm font-semibold hover:bg-zinc-200">
      Edit profile
    </button>
  ) : (
    <button
      type="button"
      onClick={() => actions.toggleFollow(social, profile.userId)}
      className={cn(
        "rounded-lg px-5 py-1.5 text-sm font-semibold",
        following ? "bg-zinc-100 hover:bg-zinc-200" : "bg-brand text-white hover:opacity-90",
      )}
    >
      {following ? "Following" : followsYou ? "Follow back" : "Follow"}
    </button>
  );

  return (
    <>
      <header className="flex gap-6 px-4 pt-4 sm:gap-20 sm:px-12 sm:pt-0">
        <Avatar profile={profile} size={150} className="hidden sm:inline-block" />
        <Avatar profile={profile} size={80} className="sm:hidden" />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-3 sm:gap-5">
            <h1 className="truncate text-xl">{profile.username}</h1>
            {button}
          </div>
          <ul className="mt-5 hidden gap-10 text-base sm:flex">
            {stats.map((s) => (
              <li key={s.label}>
                <span className="font-semibold">{formatCount(s.value)}</span> {s.label}
              </li>
            ))}
          </ul>
          <p className="mt-4 text-sm font-semibold">{profile.displayName}</p>
          {profile.bio ? <p className="mt-0.5 whitespace-pre-line text-sm">{profile.bio}</p> : null}
        </div>
      </header>

      <ul className="mt-6 flex justify-around border-y py-3 text-center text-sm sm:hidden">
        {stats.map((s) => (
          <li key={s.label}>
            <span className="block font-semibold">{formatCount(s.value)}</span>
            <span className="text-zinc-500">{s.label}</span>
          </li>
        ))}
      </ul>

      <nav className="mt-2 flex justify-center gap-14 border-t sm:mt-11">
        <TabButton on={tab === "posts"} onClick={() => setTab("posts")} icon={<Grid3x3 className="size-3.5" />}>
          Posts
        </TabButton>
        {mine ? (
          <TabButton on={tab === "saved"} onClick={() => setTab("saved")} icon={<Bookmark className="size-3.5" />}>
            Saved
          </TabButton>
        ) : null}
      </nav>

      {tab === "posts" ? (
        posts.length ? (
          <PostGrid social={social} posts={posts} />
        ) : (
          <p className="py-16 text-center text-sm text-zinc-500">{mine ? "Share your first photo with Create." : "No posts yet."}</p>
        )
      ) : saved.length ? (
        <PostGrid social={social} posts={saved} />
      ) : (
        <p className="py-16 text-center text-sm text-zinc-500">Only you can see what you save.</p>
      )}

      {mine ? <EditProfileDialog profile={profile} open={editing} onOpenChange={setEditing} /> : null}
    </>
  );
}

function TabButton({ on, onClick, icon, children }: { on: boolean; onClick: () => void; icon: React.ReactNode; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={on}
      className={cn(
        "-mt-px flex items-center gap-1.5 border-t py-4 text-xs font-semibold tracking-wide",
        on ? "border-zinc-900 text-zinc-900" : "border-transparent text-zinc-500 hover:text-zinc-700",
      )}
    >
      {icon}
      {children}
    </button>
  );
}
