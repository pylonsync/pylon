// Demo data for a new app: twelve people who post, follow each other, like,
// and comment, so the first visit opens on a feed that looks lived in.
//
// Photos live in public/images (avatars/ and posts/). Demo user ids start
// with "demo_" so they never collide with a real or guest account.
// Pure data + pure shaping; functions/seedFeed.ts is a thin wrapper.

export interface SeedProfile {
  username: string;
  displayName: string;
  bio: string;
}

export interface SeedPost {
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
  { username: "mara.bakes", displayName: "Mara Lindqvist", bio: "Sourdough, mostly. Stockholm." },
  { username: "theo.climbs", displayName: "Theo Adeyemi", bio: "Bouldering most evenings. Route setter at Blocworks." },
  { username: "ines.clay", displayName: "Inés Moreno", bio: "Wheel-thrown stoneware from a small studio in Valencia." },
  { username: "kenji.rides", displayName: "Kenji Watanabe", bio: "Gravel on weekends, commuting on weekdays." },
  { username: "lou.streets", displayName: "Louise Carter", bio: "35mm film, mostly Brooklyn." },
  { username: "sam.grows", displayName: "Sam Okafor", bio: "Allotment 14B. Tomatoes, beans, too many courgettes." },
  { username: "rosa.roasts", displayName: "Rosa Delgado", bio: "Head roaster at a small shop in Oakland." },
  { username: "biscuit.corgi", displayName: "Biscuit", bio: "Corgi. Professional napper. Managed by @nora.h" },
  { username: "nora.h", displayName: "Nora Hughes", bio: "Product designer. Biscuit's person." },
  { username: "ari.wanders", displayName: "Ari Cohen", bio: "Trains over planes." },
  { username: "dev.draws", displayName: "Dev Patel", bio: "Architect. A sketch every morning before work." },
  { username: "june.surfs", displayName: "June Tanaka", bio: "Cold water surfer on the Oregon coast." },
];

export const SEED_POSTS: SeedPost[] = [
  { key: "mara-loaf", author: "mara.bakes", caption: "Sunday loaf. 78% hydration and finally an ear I am happy with.", shape: "portrait", age: 3 },
  { key: "june-dawn", author: "june.surfs", caption: "Glassy at 6am and nobody else out. Worth the cold hands.", shape: "portrait", age: 5 },
  { key: "biscuit-couch", author: "biscuit.corgi", caption: "Guarding the couch from nothing in particular.", shape: "square", age: 8 },
  { key: "rosa-roaster", author: "rosa.roasts", caption: "New Ethiopia lot on the roaster today. Blueberry and jasmine in the cup.", shape: "square", age: 11 },
  { key: "ines-glaze", author: "ines.clay", caption: "Out of the kiln: the speckled oat glaze on a batch of mugs. Shop update Friday.", shape: "portrait", age: 14 },
  { key: "lou-crosswalk", author: "lou.streets", caption: "Bedford Ave, late afternoon. Portra 400.", shape: "portrait", age: 20 },
  { key: "dev-sketch", author: "dev.draws", caption: "Morning sketch of the old library stairs. Twenty minutes, one pen.", shape: "square", age: 26 },
  { key: "kenji-gravel", author: "kenji.rides", caption: "82 km, 1,400 m of climbing, one flat. Good day.", shape: "portrait", age: 30 },
  { key: "sam-tomatoes", author: "sam.grows", caption: "First proper harvest of the year. The yellow ones are Sungold.", shape: "square", age: 34 },
  { key: "theo-boulder", author: "theo.climbs", caption: "Set a new blue circuit this week. This one is the crux of number 6.", shape: "portrait", age: 40 },
  { key: "ari-train", author: "ari.wanders", caption: "Night train from Vienna to Venice. Woke up to this.", shape: "portrait", age: 46 },
  { key: "nora-desk", author: "nora.h", caption: "New desk setup. @biscuit.corgi approved the rug.", shape: "square", age: 52 },
  { key: "mara-buns", author: "mara.bakes", caption: "Cardamom buns for the neighbours. Kept two.", shape: "square", age: 60 },
  { key: "june-board", author: "june.surfs", caption: "Waxed and ready for tomorrow.", shape: "square", age: 70 },
  { key: "rosa-latte", author: "rosa.roasts", caption: "Tuesday latte art practice. Getting there.", shape: "square", age: 78 },
  { key: "ines-wheel", author: "ines.clay", caption: "Trimming feet on a batch of bowls. The quiet part of the week.", shape: "portrait", age: 90 },
  { key: "lou-diner", author: "lou.streets", caption: "Counter seats at the diner on Graham. Coffee refills forever.", shape: "square", age: 100 },
  { key: "biscuit-beach", author: "biscuit.corgi", caption: "First time at the beach. Sand everywhere. No regrets.", shape: "portrait", age: 110 },
  { key: "dev-model", author: "dev.draws", caption: "Card model for the community centre competition entry.", shape: "square", age: 124 },
  { key: "kenji-coffee", author: "kenji.rides", caption: "Mid-ride coffee stop. The bike rests too.", shape: "square", age: 140 },
  { key: "sam-shed", author: "sam.grows", caption: "Seed trays in the shed. Kale, chard, and far too much basil.", shape: "portrait", age: 155 },
  { key: "theo-chalk", author: "theo.climbs", caption: "Chalk bag, tape, and a very tired pair of shoes.", shape: "square", age: 170 },
  { key: "ari-market", author: "ari.wanders", caption: "Morning market in Palermo. I bought too many lemons.", shape: "square", age: 190 },
  { key: "nora-sketches", author: "nora.h", caption: "Paper first, screens later. Wireframes for the booking flow.", shape: "square", age: 210 },
  { key: "mara-starter", author: "mara.bakes", caption: "Meet Olof, the starter. Eleven years old this month.", shape: "portrait", age: 240 },
  { key: "june-van", author: "june.surfs", caption: "Home for the weekend.", shape: "square", age: 280 },
  { key: "ines-studio", author: "ines.clay", caption: "The studio shelves on a good week.", shape: "portrait", age: 320 },
  { key: "lou-rain", author: "lou.streets", caption: "Rain on the J train platform.", shape: "portrait", age: 360 },
];

export const SEED_COMMENTS: SeedComment[] = [
  { post: "mara-loaf", author: "rosa.roasts", text: "That crumb. Trading you a bag of the new Ethiopia for one.", after: 0.5 },
  { post: "mara-loaf", author: "sam.grows", text: "What flour are you using?", after: 1 },
  { post: "mara-loaf", author: "mara.bakes", text: "@sam.grows 80% bread flour, 20% whole wheat from the mill down the road.", after: 1.5 },
  { post: "june-dawn", author: "kenji.rides", text: "Unreal colours.", after: 1 },
  { post: "june-dawn", author: "ari.wanders", text: "Which beach is this?", after: 2 },
  { post: "biscuit-couch", author: "nora.h", text: "He has not moved in three hours.", after: 0.2 },
  { post: "biscuit-couch", author: "lou.streets", text: "A very serious job.", after: 2 },
  { post: "biscuit-couch", author: "theo.climbs", text: "Those ears.", after: 3 },
  { post: "rosa-roaster", author: "mara.bakes", text: "Saving me a bag?", after: 1 },
  { post: "rosa-roaster", author: "rosa.roasts", text: "@mara.bakes already set aside.", after: 1.2 },
  { post: "ines-glaze", author: "dev.draws", text: "The speckle on that glaze is lovely.", after: 2 },
  { post: "ines-glaze", author: "nora.h", text: "Setting an alarm for Friday.", after: 3 },
  { post: "lou-crosswalk", author: "ari.wanders", text: "The light in this one.", after: 4 },
  { post: "dev-sketch", author: "ines.clay", text: "Twenty minutes? Show-off.", after: 1 },
  { post: "kenji-gravel", author: "june.surfs", text: "Where was the flat?", after: 2 },
  { post: "kenji-gravel", author: "kenji.rides", text: "@june.surfs about 5 km from the end, of course.", after: 3 },
  { post: "sam-tomatoes", author: "mara.bakes", text: "Tomato and bread season.", after: 1 },
  { post: "theo-boulder", author: "kenji.rides", text: "Looks sandbagged.", after: 2 },
  { post: "theo-boulder", author: "theo.climbs", text: "@kenji.rides it is a fair blue. Mostly.", after: 2.5 },
  { post: "ari-train", author: "lou.streets", text: "Adding this to the list.", after: 5 },
  { post: "nora-desk", author: "dev.draws", text: "Where is the lamp from?", after: 1 },
  { post: "mara-buns", author: "rosa.roasts", text: "Recipe please.", after: 2 },
  { post: "biscuit-beach", author: "june.surfs", text: "Surf lessons next.", after: 1 },
  { post: "dev-model", author: "ines.clay", text: "Good luck with the entry.", after: 3 },
  { post: "sam-shed", author: "mara.bakes", text: "Pesto for the whole street then.", after: 4 },
  { post: "mara-starter", author: "biscuit.corgi", text: "Can I eat Olof.", after: 2 },
  { post: "mara-starter", author: "mara.bakes", text: "@biscuit.corgi no.", after: 2.2 },
];

/** Who each demo user follows, as username → usernames. */
export const SEED_FOLLOWS: Record<string, string[]> = {
  "mara.bakes": ["rosa.roasts", "sam.grows", "ines.clay", "biscuit.corgi", "lou.streets"],
  "theo.climbs": ["kenji.rides", "june.surfs", "ari.wanders", "biscuit.corgi"],
  "ines.clay": ["dev.draws", "mara.bakes", "nora.h", "lou.streets"],
  "kenji.rides": ["theo.climbs", "june.surfs", "rosa.roasts", "ari.wanders"],
  "lou.streets": ["ari.wanders", "dev.draws", "biscuit.corgi", "ines.clay"],
  "sam.grows": ["mara.bakes", "rosa.roasts", "biscuit.corgi"],
  "rosa.roasts": ["mara.bakes", "kenji.rides", "ines.clay", "sam.grows"],
  "biscuit.corgi": ["nora.h", "june.surfs"],
  "nora.h": ["biscuit.corgi", "dev.draws", "ines.clay", "lou.streets", "mara.bakes"],
  "ari.wanders": ["lou.streets", "june.surfs", "kenji.rides", "theo.climbs"],
  "dev.draws": ["ines.clay", "nora.h", "lou.streets"],
  "june.surfs": ["kenji.rides", "biscuit.corgi", "ari.wanders", "theo.climbs"],
};

export const demoUserId = (username: string) => `demo_${username.replace(/\./g, "_")}`;
export const avatarPath = (username: string) => `/images/avatars/${username}.jpg`;
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
      avatarUrl: avatarPath(p.username),
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
