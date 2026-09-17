import { describe, expect, test } from "bun:test";
import {
  formatCount,
  homeFeed,
  isPostImageUrl,
  normalizeUsername,
  recentPosters,
  splitMentions,
  starterUsername,
  suggestions,
  timeAgo,
  usernameProblem,
  type Follow,
  type Post,
  type Profile,
} from "../lib/social";
import { SEED_COMMENTS, SEED_FOLLOWS, SEED_POSTS, SEED_PROFILES, likersOf, shapeSeed } from "../lib/seed";

const NOW = Date.parse("2026-09-17T12:00:00Z");
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString();
const post = (id: string, authorId: string, h: number): Post => ({ id, authorId, imageUrl: `/images/posts/${id}.jpg`, createdAt: hoursAgo(h) });
const profile = (userId: string, username: string): Profile => ({ id: `p_${userId}`, userId, username, displayName: username });
const follow = (followerId: string, followingId: string): Follow => ({ id: `${followerId}>${followingId}`, followerId, followingId });

describe("usernames", () => {
  test("accepts the usual shapes and explains refusals", () => {
    for (const ok of ["mara.bakes", "june_surfs", "abc", "a1.b2_c3"]) expect(usernameProblem(ok)).toBeNull();
    expect(usernameProblem("ab")).toContain("at least");
    expect(usernameProblem("x".repeat(31))).toContain("at most");
    expect(usernameProblem("has space")).toContain("letters");
    expect(usernameProblem("Upper")).toContain("letters");
    expect(usernameProblem(".dot")).toContain("start or end");
    expect(usernameProblem("dot.")).toContain("start or end");
    expect(usernameProblem("two..dots")).toContain("two periods");
  });

  test("normalizes input and starts new accounts on a valid, stable name", () => {
    expect(normalizeUsername("  @Mara.Bakes ")).toBe("mara.bakes");
    const name = starterUsername("guest_4f2a9c");
    expect(name).toBe(starterUsername("guest_4f2a9c"));
    expect(name).not.toBe(starterUsername("guest_4f2a9d"));
    expect(usernameProblem(name)).toBeNull();
  });
});

describe("labels", () => {
  test("counts", () => {
    expect(formatCount(0)).toBe("0");
    expect(formatCount(1204)).toBe("1,204");
    expect(formatCount(12_500)).toBe("12.5K");
    expect(formatCount(40_000)).toBe("40K");
    expect(formatCount(3_400_000)).toBe("3.4M");
  });

  test("ages", () => {
    expect(timeAgo(hoursAgo(0), NOW)).toBe("now");
    expect(timeAgo(new Date(NOW - 5 * 60_000).toISOString(), NOW)).toBe("5m");
    expect(timeAgo(hoursAgo(3), NOW)).toBe("3h");
    expect(timeAgo(hoursAgo(50), NOW)).toBe("2d");
    expect(timeAgo(hoursAgo(24 * 15), NOW)).toBe("2w");
    expect(timeAgo("2026-03-04T10:00:00Z", NOW)).toBe("Mar 4");
    expect(timeAgo("2024-03-04T10:00:00Z", NOW)).toBe("Mar 4, 2024");
    expect(timeAgo("not a date", NOW)).toBe("");
  });

  test("mentions become links only when the name is valid", () => {
    expect(splitMentions("thanks @nora.h and @biscuit.corgi.")).toEqual([
      { kind: "text", text: "thanks " },
      { kind: "mention", username: "nora.h" },
      { kind: "text", text: " and " },
      { kind: "mention", username: "biscuit.corgi" },
      { kind: "text", text: "." },
    ]);
    expect(splitMentions("email me@x or @ab")).toEqual([{ kind: "text", text: "email me@x or @ab" }]);
  });

  test("post images: uploads, public files, or https only", () => {
    expect(isPostImageUrl("/api/files/abc123")).toBe(true);
    expect(isPostImageUrl("/images/posts/a.jpg")).toBe(true);
    expect(isPostImageUrl("https://cdn.example.com/a.jpg")).toBe(true);
    expect(isPostImageUrl("http://cdn.example.com/a.jpg")).toBe(false);
    expect(isPostImageUrl("javascript:alert(1)")).toBe(false);
    expect(isPostImageUrl("/images/../app.ts")).toBe(false);
    expect(isPostImageUrl("")).toBe(false);
  });
});

describe("who sees what", () => {
  const posts = [post("a", "u1", 1), post("b", "u2", 2), post("c", "me", 3), post("d", "u3", 100)];

  test("someone who follows nobody sees everything", () => {
    const feed = homeFeed(posts, new Set(), "me");
    expect(feed.everyone).toBe(true);
    expect(feed.posts.map((p) => p.id)).toEqual(["a", "b", "c", "d"]);
  });

  test("otherwise the people you follow and yourself, newest first", () => {
    const feed = homeFeed(posts, new Set(["u2", "u3"]), "me");
    expect(feed.everyone).toBe(false);
    expect(feed.posts.map((p) => p.id)).toEqual(["b", "c", "d"]);
  });

  test("suggestions skip you and people you follow, most followed first", () => {
    const profiles = [profile("me", "me.me"), profile("u1", "zed"), profile("u2", "amy"), profile("u3", "bob")];
    const follows = [follow("u1", "u3"), follow("u2", "u3"), follow("u3", "u1"), follow("me", "u2")];
    const out = suggestions(profiles, follows, new Set(["u2"]), "me");
    expect(out.map((p) => p.userId)).toEqual(["u3", "u1"]);
  });

  test("recent posters are unique and inside the window", () => {
    const list = [post("a", "u1", 1), post("b", "u1", 2), post("c", "u2", 5), post("d", "u3", 60)];
    expect(recentPosters(list, NOW, 48)).toEqual(["u1", "u2"]);
  });
});

describe("demo data", () => {
  const usernames = new Set(SEED_PROFILES.map((p) => p.username));

  test("every username is valid and every reference points at a real row", () => {
    for (const u of usernames) expect(usernameProblem(u)).toBeNull();
    const keys = new Set(SEED_POSTS.map((p) => p.key));
    expect(keys.size).toBe(SEED_POSTS.length);
    for (const p of SEED_POSTS) expect(usernames.has(p.author)).toBe(true);
    for (const c of SEED_COMMENTS) {
      expect(keys.has(c.post)).toBe(true);
      expect(usernames.has(c.author)).toBe(true);
    }
    for (const [from, tos] of Object.entries(SEED_FOLLOWS)) {
      expect(usernames.has(from)).toBe(true);
      for (const to of tos) {
        expect(usernames.has(to)).toBe(true);
        expect(to).not.toBe(from);
      }
    }
  });

  test("likes: four to eleven people, never the author, no one twice", () => {
    for (const p of SEED_POSTS) {
      const likers = likersOf(p);
      expect(likers.length).toBeGreaterThanOrEqual(4);
      expect(likers.length).toBeLessThanOrEqual(11);
      expect(likers).not.toContain(p.author);
      expect(new Set(likers).size).toBe(likers.length);
    }
  });

  test("shaped rows are dated in the past relative to now", () => {
    const seed = shapeSeed(NOW);
    expect(seed.profiles).toHaveLength(SEED_PROFILES.length);
    for (const row of [...seed.posts.map((p) => p.row), ...seed.likes.map((l) => l.row), ...seed.comments.map((c) => c.row)]) {
      expect(Date.parse(String(row.createdAt))).toBeLessThanOrEqual(NOW);
    }
    // A comment never predates its post.
    const postTime = new Map(seed.posts.map((p) => [p.key, Date.parse(String(p.row.createdAt))]));
    for (const c of seed.comments) expect(Date.parse(String(c.row.createdAt))).toBeGreaterThanOrEqual(postTime.get(c.post)!);
  });
});
