"use client";

import React from "react";
import { Link } from "@pylonsync/react";
import { homeFeed, plural, recentPosters, suggestions } from "@/lib/social";
import { SITE } from "@/lib/site";
import { Avatar } from "./avatar";
import { PostCard } from "./post-card";
import { SocialGate } from "./session";
import { actions, useSocial, type Social } from "./use-social";

export function HomeScreen() {
  return (
    <SocialGate fallback={<HomeFrame />}>
      <LiveHome />
    </SocialGate>
  );
}

function LiveHome() {
  const social = useSocial();
  const feed = homeFeed(social.posts, social.following, social.me);
  const recent = recentPosters(social.posts).slice(0, 8);

  return (
    <HomeFrame
      rail={<Rail social={social} />}
      recent={
        recent.length ? (
          <ul className="flex gap-3 overflow-x-auto px-3 pb-4 pt-2 sm:px-0">
            {recent.map((userId) => {
              const profile = social.profileByUser.get(userId);
              if (!profile) return null;
              return (
                <li key={userId} className="w-[74px] shrink-0">
                  <Link href={`/u/${profile.username}`} className="flex flex-col items-center gap-1">
                    <Avatar profile={profile} size={62} ring />
                    <span className="w-full truncate text-center text-xs">{profile.username}</span>
                  </Link>
                </li>
              );
            })}
          </ul>
        ) : null
      }
    >
      {social.loading ? (
        <FeedSkeleton />
      ) : feed.posts.length === 0 ? (
        <p className="px-3 py-16 text-center text-sm text-zinc-500">No posts yet. Share the first photo with Create.</p>
      ) : (
        <>
          {feed.everyone ? (
            <p className="px-3 pb-2 text-sm text-zinc-500 sm:px-0">Posts from everyone. Follow people to see only theirs.</p>
          ) : null}
          <div className="flex flex-col gap-4">
            {feed.posts.map((post) => (
              <PostCard key={post.id} social={social} post={post} />
            ))}
          </div>
        </>
      )}
    </HomeFrame>
  );
}

function HomeFrame({ children, rail, recent }: { children?: React.ReactNode; rail?: React.ReactNode; recent?: React.ReactNode }) {
  return (
    <div className="mx-auto flex max-w-[935px] justify-center gap-16 pt-2 sm:pt-6">
      <div className="w-full max-w-[470px]">
        {recent}
        {children ?? <FeedSkeleton />}
      </div>
      <aside className="hidden w-[320px] shrink-0 pt-6 lg:block">{rail}</aside>
    </div>
  );
}

function Rail({ social }: { social: Social }) {
  const people = suggestions(social.profiles, social.follows, social.following, social.me, 5);
  const me = social.myProfile;
  return (
    <div className="sticky top-8">
      {me ? (
        <div className="flex items-center gap-3">
          <Link href={`/u/${me.username}`}>
            <Avatar profile={me} size={44} />
          </Link>
          <div className="min-w-0 flex-1 text-sm">
            <Link href={`/u/${me.username}`} className="block truncate font-semibold">
              {me.username}
            </Link>
            <span className="block truncate text-zinc-500">{me.displayName}</span>
          </div>
          <Link href={`/u/${me.username}`} className="text-xs font-semibold text-brand hover:text-zinc-900">
            Edit profile
          </Link>
        </div>
      ) : null}

      {people.length ? (
        <>
          <div className="mb-2 mt-6 flex items-center justify-between text-sm">
            <span className="font-semibold text-zinc-500">Suggested for you</span>
            <Link href="/explore" className="text-xs font-semibold hover:text-zinc-500">
              See all
            </Link>
          </div>
          <ul className="flex flex-col gap-3">
            {people.map((p) => {
              const followers = social.followerCount.get(p.userId) ?? 0;
              return (
                <li key={p.id} className="flex items-center gap-3">
                  <Link href={`/u/${p.username}`}>
                    <Avatar profile={p} size={32} />
                  </Link>
                  <div className="min-w-0 flex-1 text-sm">
                    <Link href={`/u/${p.username}`} className="block truncate font-semibold">
                      {p.username}
                    </Link>
                    <span className="block truncate text-xs text-zinc-500">{plural(followers, "follower")}</span>
                  </div>
                  <button
                    type="button"
                    onClick={() => actions.toggleFollow(social, p.userId)}
                    className="text-xs font-semibold text-brand hover:text-zinc-900"
                  >
                    Follow
                  </button>
                </li>
              );
            })}
          </ul>
        </>
      ) : null}

      <p className="mt-8 text-xs text-zinc-400">
        {SITE.name} · Built with{" "}
        <a href="https://pylonsync.com" className="hover:underline">
          Pylon
        </a>
      </p>
    </div>
  );
}

function FeedSkeleton() {
  return (
    <div className="flex flex-col gap-6" aria-hidden>
      {[0, 1].map((i) => (
        <div key={i}>
          <div className="flex items-center gap-2.5 px-3 py-3 sm:px-0">
            <span className="size-8 rounded-full bg-zinc-100" />
            <span className="h-3 w-28 rounded bg-zinc-100" />
          </div>
          <div className="aspect-square w-full bg-zinc-100 sm:rounded-[4px]" />
        </div>
      ))}
    </div>
  );
}
