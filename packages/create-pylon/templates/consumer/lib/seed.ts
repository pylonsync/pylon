// Demo data for a new app: eight people who post, follow each other, like,
// and comment, so the first visit opens on a feed with content in it.
//
// Post photos live in public/images/posts and the server serves them at
// /images/posts/<key>.jpg. Demo people have no profile photo, so the app
// draws their initial. Demo user ids start with "demo_" so they never
// collide with a real or guest account.
// Pure data + pure shaping; functions/seedFeed.ts is a thin wrapper.

export interface SeedProfile {
  username: string;
  displayName: string;
  bio: string;
}

export interface SeedPost {
  /** File name under public/images/posts, without `.jpg`. */
  key: string;
  author: string;
  caption: string;
  shape: "square" | "portrait";
  /** Hours ago. */
  age: number;
}

export interface SeedComment {
  post: string;
  author: string;
  text: string;
  /** Hours after the post. */
  after: number;
}

export const SEED_PROFILES: SeedProfile[] = [
  { username: "maya.rivera", displayName: "Maya Rivera", bio: "Travel photographer. Currently in Lisbon." },
  { username: "jordan.eats", displayName: "Jordan Brooks", bio: "Cooking at home, eating out, writing it all down." },
  { username: "priya.home", displayName: "Priya Shah", bio: "Interior designer. Small rooms, good light." },
  { username: "kenji.mori", displayName: "Kenji Mori", bio: "Architecture and night walks. Tokyo." },
  { username: "elena.runs", displayName: "Elena Berg", bio: "Trail runner. Mountains most weekends." },
  { username: "sam.and.juno", displayName: "Sam Whitaker", bio: "Juno is the golden one." },
  { username: "amara.o", displayName: "Amara Okafor", bio: "Coffee, markets, and early mornings." },
  { username: "leo.haddad", displayName: "Leo Haddad", bio: "Road trips and sunsets. Film when I can." },
];

export const SEED_POSTS: SeedPost[] = [
  { key: "lisbon", author: "maya.rivera", caption: "Tram 28 at golden hour. Waited twenty minutes for this one and it was worth it.", shape: "square", age: 1.5 },
  { key: "coffee", author: "amara.o", caption: "Flat white and a croissant before the market opens.", shape: "square", age: 3 },
  { key: "dog", author: "sam.and.juno", caption: "Juno found the tall grass again.", shape: "square", age: 5 },
  { key: "run", author: "elena.runs", caption: "Sunrise on the ridge. 18 km, 900 m up, legs gone.", shape: "square", age: 8 },
  { key: "ramen", author: "jordan.eats", caption: "Tonkotsu from the new place on 5th. Broth cooked for 14 hours and you can tell.", shape: "square", age: 11 },
  { key: "arch", author: "kenji.mori", caption: "Curves and shadows. The museum extension at noon.", shape: "square", age: 14 },
  { key: "sunset", author: "leo.haddad", caption: "Big Sur, last light. 30 second exposure.", shape: "square", age: 20 },
];

export const SEED_COMMENTS: SeedComment[] = [
  { post: "lisbon", author: "leo.haddad", text: "The light on those tiles. Which street is this?", after: 0.3 },
  { post: "lisbon", author: "maya.rivera", text: "@leo.haddad Rua da Conceição, just before it turns up to the cathedral.", after: 0.6 },
  { post: "lisbon", author: "amara.o", text: "Adding this to the list for May.", after: 0.9 },
  { post: "coffee", author: "jordan.eats", text: "That rosetta is perfect.", after: 0.5 },
  { post: "coffee", author: "priya.home", text: "Where is this? I need that table.", after: 1 },
  { post: "dog", author: "elena.runs", text: "Juno looks so happy.", after: 0.4 },
  { post: "dog", author: "amara.o", text: "Best dog on this app.", after: 1.2 },
  { post: "run", author: "kenji.mori", text: "Incredible view. How early did you start?", after: 1 },
  { post: "run", author: "elena.runs", text: "@kenji.mori headlamps on at 4:30.", after: 1.5 },
  { post: "ramen", author: "maya.rivera", text: "Going this week.", after: 2 },
  { post: "arch", author: "priya.home", text: "Those shadows are unreal.", after: 1 },
  { post: "sunset", author: "sam.and.juno", text: "Colors like this make me want to drive up the coast tonight.", after: 2 },
];

/** Who each demo user follows, as username → usernames. */
export const SEED_FOLLOWS: Record<string, string[]> = {
  "maya.rivera": ["leo.haddad", "amara.o", "priya.home", "kenji.mori"],
  "jordan.eats": ["amara.o", "maya.rivera", "sam.and.juno"],
  "priya.home": ["kenji.mori", "maya.rivera", "amara.o", "elena.runs"],
  "kenji.mori": ["priya.home", "leo.haddad", "elena.runs"],
  "elena.runs": ["sam.and.juno", "leo.haddad", "maya.rivera", "kenji.mori"],
  "sam.and.juno": ["elena.runs", "jordan.eats", "amara.o"],
  "amara.o": ["jordan.eats", "maya.rivera", "priya.home", "sam.and.juno", "elena.runs"],
  "leo.haddad": ["maya.rivera", "elena.runs", "kenji.mori"],
};

export const demoUserId = (username: string) => `demo_${username.replace(/\./g, "_")}`;
export const postImagePath = (key: string) => `/images/posts/${key}.jpg`;

/**
 * Which demo users like a post: a stable pick from everyone but the author,
 * between four and eleven people, so counts differ from post to post.
 */
export function likersOf(post: SeedPost): string[] {
  const others = SEED_PROFILES.map((p) => p.username).filter((u) => u !== post.author);
  let h = 2166136261;
  for (let i = 0; i < post.key.length; i++) {
    h ^= post.key.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  const count = 4 + ((h >>> 0) % (others.length - 3));
  const start = (h >>> 8) % others.length;
  return Array.from({ length: count }, (_, i) => others[(start + i) % others.length]);
}

export interface ShapedSeed {
  profiles: Record<string, unknown>[];
  posts: Array<{ key: string; row: Record<string, unknown> }>;
  likes: Array<{ post: string; row: Record<string, unknown> }>;
  comments: Array<{ post: string; row: Record<string, unknown> }>;
  follows: Record<string, unknown>[];
}

export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const hoursAgo = (h: number) => new Date(now - h * 3_600_000).toISOString();
  const postAge = new Map(SEED_POSTS.map((p) => [p.key, p.age]));

  return {
    profiles: SEED_PROFILES.map((p, i) => ({
      userId: demoUserId(p.username),
      username: p.username,
      displayName: p.displayName,
      bio: p.bio,
      avatarUrl: null,
      createdAt: hoursAgo(2000 - i * 40),
    })),
    posts: SEED_POSTS.map((p) => ({
      key: p.key,
      row: {
        authorId: demoUserId(p.author),
        imageUrl: postImagePath(p.key),
        caption: p.caption,
        shape: p.shape,
        createdAt: hoursAgo(p.age),
      },
    })),
    likes: SEED_POSTS.flatMap((p) =>
      likersOf(p).map((u, i) => ({
        post: p.key,
        row: { userId: demoUserId(u), createdAt: hoursAgo(Math.max(0.1, p.age - 0.5 - i * 0.7)) },
      })),
    ),
    comments: SEED_COMMENTS.map((c) => ({
      post: c.post,
      row: {
        authorId: demoUserId(c.author),
        text: c.text,
        createdAt: hoursAgo(Math.max(0.05, (postAge.get(c.post) ?? 0) - c.after)),
      },
    })),
    follows: Object.entries(SEED_FOLLOWS).flatMap(([from, tos], i) =>
      tos.map((to) => ({ followerId: demoUserId(from), followingId: demoUserId(to), createdAt: hoursAgo(1500 - i * 30) })),
    ),
  };
}
