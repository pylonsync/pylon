import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { SEED_COMMENTS, SEED_FOLLOWS, SEED_POSTS, SEED_PROFILES, shapeSeed } from "../lib/seed";

const PUBLIC = join(import.meta.dir, "..", "public");

/** Every image path the seed writes that the server must serve from public/. */
function localImages(): string[] {
  const seed = shapeSeed(0);
  const urls = [
    ...seed.posts.map((p) => p.row.imageUrl),
    ...seed.profiles.map((p) => p.avatarUrl),
  ];
  return urls.filter((u): u is string => typeof u === "string" && u.startsWith("/"));
}

describe("demo seed", () => {
  test("every local image it references exists in public/", () => {
    const images = localImages();
    expect(images.length).toBeGreaterThan(0);
    const missing = images.filter((url) => !existsSync(join(PUBLIC, url)));
    expect(missing).toEqual([]);
  });

  test("posts, comments, and follows only name demo people", () => {
    const people = new Set(SEED_PROFILES.map((p) => p.username));
    const posts = new Set(SEED_POSTS.map((p) => p.key));
    for (const p of SEED_POSTS) expect(people.has(p.author)).toBe(true);
    for (const c of SEED_COMMENTS) {
      expect(people.has(c.author)).toBe(true);
      expect(posts.has(c.post)).toBe(true);
    }
    for (const [from, tos] of Object.entries(SEED_FOLLOWS)) {
      expect(people.has(from)).toBe(true);
      for (const to of tos) expect(people.has(to)).toBe(true);
    }
  });
});
