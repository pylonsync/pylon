"use client";

import { useMemo } from "react";
import { db } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import {
  countBy,
  followingOf,
  groupBy,
  newestFirst,
  type Comment,
  type Follow,
  type Like,
  type Post,
  type Profile,
  type Save,
} from "@/lib/social";

/**
 * Everything the screens read, joined once. Each `db.useQuery` is live, so a
 * like, comment, or follow from another tab shows up here without a reload.
 * Fine for a demo-sized network; a large one would query per screen with
 * `where` and `limit` instead of reading every row.
 */
export function useSocial() {
  const { userId } = useAuth();
  const profiles = db.useQuery<Profile>("Profile");
  const posts = db.useQuery<Post>("Post");
  const likes = db.useQuery<Like>("Like");
  const comments = db.useQuery<Comment>("Comment");
  const follows = db.useQuery<Follow>("Follow");
  const saves = db.useQuery<Save>("Save");

  return useMemo(() => {
    const me = userId ?? null;
    const profileByUser = new Map(profiles.data.map((p) => [p.userId, p]));
    const profileByUsername = new Map(profiles.data.map((p) => [p.username, p]));
    const myLikes = new Map(likes.data.filter((l) => l.userId === me).map((l) => [l.postId, l]));
    const mySaves = new Map(saves.data.filter((s) => s.userId === me).map((s) => [s.postId, s]));
    const myFollows = new Map(follows.data.filter((f) => f.followerId === me).map((f) => [f.followingId, f]));
    return {
      me,
      myProfile: me ? profileByUser.get(me) ?? null : null,
      loading: posts.loading && posts.data.length === 0,
      profiles: profiles.data,
      posts: newestFirst(posts.data),
      follows: follows.data,
      profileByUser,
      profileByUsername,
      likeCount: countBy(likes.data, (l) => l.postId),
      commentsByPost: groupBy(newestFirst(comments.data).reverse(), (c) => c.postId),
      postsByAuthor: groupBy(newestFirst(posts.data), (p) => p.authorId),
      followerCount: countBy(follows.data, (f) => f.followingId),
      followingCount: countBy(follows.data, (f) => f.followerId),
      following: followingOf(follows.data, me),
      myLikes,
      mySaves,
      myFollows,
    };
  }, [userId, profiles.data, posts.data, posts.loading, likes.data, comments.data, follows.data, saves.data]);
}

export type Social = ReturnType<typeof useSocial>;

/** Writes. Likes, saves, and follows are direct; posts and comments go through functions. */
export const actions = {
  toggleLike(social: Social, postId: string) {
    const mine = social.myLikes.get(postId);
    if (mine) void db.delete("Like", mine.id);
    else if (social.me) void db.insert("Like", { postId, userId: social.me });
  },
  like(social: Social, postId: string) {
    if (social.me && !social.myLikes.has(postId)) void db.insert("Like", { postId, userId: social.me });
  },
  toggleSave(social: Social, postId: string) {
    const mine = social.mySaves.get(postId);
    if (mine) void db.delete("Save", mine.id);
    else if (social.me) void db.insert("Save", { postId, userId: social.me });
  },
  toggleFollow(social: Social, userId: string) {
    if (!social.me || userId === social.me) return;
    const mine = social.myFollows.get(userId);
    if (mine) void db.delete("Follow", mine.id);
    else void db.insert("Follow", { followingId: userId, followerId: social.me });
  },
};
