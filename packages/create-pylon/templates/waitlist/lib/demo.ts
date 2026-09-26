// Demo signups for local development.
//
// `pylon dev` fills an empty waitlist with about 180 fictional signups spread
// over the last 30 days, so the counter, chart, and list have data. A deploy
// does not seed: `demoDataEnabled` is true only when PYLON_DEMO_DATA is on, or
// when it is unset and the process runs under `pylon dev`. Set
// PYLON_DEMO_DATA=0 to turn it off in development too.
//
// Pure data + pure shaping, so functions/seedDemo.ts stays a thin wrapper and
// this file is testable without a server.

const ON = new Set(["1", "true", "yes", "on"]);
const OFF = new Set(["0", "false", "no", "off"]);

/**
 * True when demo rows may be written. An explicit PYLON_DEMO_DATA wins.
 * Otherwise it needs both PYLON_DEV_MODE on and PYLON_DEV_WATCH_DIR set. Only
 * `pylon dev` sets PYLON_DEV_WATCH_DIR; `pylon start` and the Docker image do
 * not, and the image turns PYLON_DEV_MODE on by default, so dev mode alone is
 * not proof of a local machine.
 */
export function demoDataEnabled(env: Record<string, string | undefined>): boolean {
  const explicit = env.PYLON_DEMO_DATA?.trim().toLowerCase();
  if (explicit && ON.has(explicit)) return true;
  if (explicit && OFF.has(explicit)) return false;
  const dev = env.PYLON_DEV_MODE?.trim().toLowerCase();
  const devMode = dev === "1" || dev === "true";
  return devMode && Boolean(env.PYLON_DEV_WATCH_DIR?.trim());
}

const FIRST = [
  "priya", "marcus", "lena", "tomas", "aisha", "ben", "hana", "diego", "sofia", "kwame",
  "ingrid", "raj", "noor", "felix", "maya", "owen", "yuki", "carmen", "theo", "amara",
  "jonas", "leila", "sam", "ines", "dev", "clara", "malik", "rosa", "ethan", "zara",
];
const LAST = [
  "nair", "bell", "fischer", "alvarez", "khan", "osei", "sato", "ruiz", "marin", "mensah",
  "berg", "patel", "haddad", "wagner", "chen", "reilly", "tanaka", "vega", "park", "okafor",
];
// Fictional company domains on the reserved .example TLD, so no demo address
// can reach a real inbox.
const DOMAINS = [
  "fieldnote.example", "northwind.example", "quarry.example", "bluefin.example", "wren.example",
  "copperline.example", "tidewater.example", "larkspur.example", "kestrel.example", "orchardlabs.example",
  "sable.example", "ironwood.example", "halcyon.example", "pinecrest.example", "vantage.example",
];

/** Small deterministic PRNG (mulberry32) so the demo set is stable per seed. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export interface DemoSignup {
  email: string;
  createdAt: string;
}

/**
 * About 180 signups over the 30 days before `nowMs`, rising toward today.
 * Emails are unique. Every timestamp is in the past.
 */
export function demoSignups(nowMs: number, seed = 7): DemoSignup[] {
  const rand = rng(seed);
  const out: DemoSignup[] = [];
  const seen = new Set<string>();
  const DAY = 86_400_000;
  for (let daysAgo = 29; daysAgo >= 0; daysAgo--) {
    // Growth curve: ~2/day a month ago, ~10/day this week, plus noise.
    const base = 2 + Math.round((29 - daysAgo) * 0.28);
    const count = Math.max(0, base + Math.round((rand() - 0.5) * 4));
    for (let i = 0; i < count; i++) {
      const first = FIRST[Math.floor(rand() * FIRST.length)];
      const last = LAST[Math.floor(rand() * LAST.length)];
      const domain = DOMAINS[Math.floor(rand() * DOMAINS.length)];
      const shape = rand();
      const local = shape < 0.5 ? `${first}.${last}` : shape < 0.8 ? `${first}` : `${first[0]}${last}`;
      const email = `${local}@${domain}`;
      if (seen.has(email)) continue;
      seen.add(email);
      // A random minute inside that day, never later than `nowMs`.
      const offset = Math.floor(rand() * DAY);
      const ts = Math.min(nowMs - 60_000, nowMs - daysAgo * DAY - offset);
      out.push({ email, createdAt: new Date(ts).toISOString() });
    }
  }
  return out.sort((a, b) => (a.createdAt < b.createdAt ? -1 : 1));
}
