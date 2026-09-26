// Workspace loaded on first boot by functions/joinChat.ts. The channels are
// always created. With `includeDemoConversation` on, four demo people (a team
// shipping an offline-maps update for a trail-running app) and their messages
// are added too. Set it to false to start with empty channels.
//
// `minutesAgo` is relative to the moment the seed runs, so the conversation
// always reads as yesterday plus this morning.

export const includeDemoConversation = true;

export interface SeedChannel {
  slug: string;
  name: string;
  topic: string;
}

export interface SeedPerson {
  /** Stable author id for seeded rows. Real users have session ids instead. */
  userId: string;
  name: string;
  hue: number;
}

export interface SeedMessage {
  channel: string;
  author: string;
  minutesAgo: number;
  text: string;
}

export const seedChannels: SeedChannel[] = [
  { slug: "general", name: "general", topic: "Anything that doesn't fit elsewhere" },
  { slug: "launch", name: "launch", topic: "Ridgeline 2.4 ships Thursday at 10:00" },
  { slug: "design", name: "design", topic: "Screens, icons, and App Store art" },
];

export const seedPeople: SeedPerson[] = [
  { userId: "seed_maya", name: "Maya Chen", hue: 28 },
  { userId: "seed_theo", name: "Theo Okafor", hue: 250 },
  { userId: "seed_priya", name: "Priya Raman", hue: 150 },
  { userId: "seed_jonas", name: "Jonas Weber", hue: 330 },
];

const DAY = 24 * 60;

export const seedMessages: SeedMessage[] = [
  // #launch, yesterday afternoon
  { channel: "launch", author: "seed_priya", minutesAgo: DAY + 190, text: "Status check for 2.4. We still have three open items: the iPad crash, the App Store screenshots, and the release notes." },
  { channel: "launch", author: "seed_maya", minutesAgo: DAY + 186, text: "The iPad crash is in the tile cache. When you download a region bigger than about 2 GB, the eviction pass runs on the main thread and the watchdog kills the app." },
  { channel: "launch", author: "seed_maya", minutesAgo: DAY + 185, text: "Moving eviction to a background queue now. Should have a build tonight." },
  { channel: "launch", author: "seed_jonas", minutesAgo: DAY + 171, text: "I can reproduce it every time on the 10th gen iPad with the Cascades region. Happy to run it again once the build is up." },
  { channel: "launch", author: "seed_priya", minutesAgo: DAY + 168, text: "Great. If it slips past Wednesday noon we move the release to next week. I'd rather not ship offline maps that crash offline." },
  { channel: "launch", author: "seed_theo", minutesAgo: DAY + 122, text: "Screenshots are 80% done. Waiting on a clean capture of the elevation profile with the new colors." },
  { channel: "launch", author: "seed_maya", minutesAgo: DAY + 34, text: "TestFlight build 412 is up. Eviction runs in the background, and I added a progress bar so a big cleanup doesn't look frozen." },
  { channel: "launch", author: "seed_jonas", minutesAgo: DAY + 21, text: "Installing now." },

  // #launch, this morning
  { channel: "launch", author: "seed_jonas", minutesAgo: 58, text: "Build 412 survived six full downloads of the Cascades region on the iPad. No crash, memory stays under 600 MB." },
  { channel: "launch", author: "seed_jonas", minutesAgo: 57, text: "One small thing: the progress bar says 100% for about four seconds before it disappears." },
  { channel: "launch", author: "seed_maya", minutesAgo: 49, text: "That's the index rebuild after the last tile lands. I'll change the label to \"Finishing up\" for that step instead of hiding it." },
  { channel: "launch", author: "seed_priya", minutesAgo: 44, text: "Perfect. That takes the crash off the list 🎉" },
  { channel: "launch", author: "seed_priya", minutesAgo: 43, text: "Remaining: screenshots (Theo) and release notes (me). Submitting for review Wednesday at 3." },
  { channel: "launch", author: "seed_theo", minutesAgo: 12, text: "Screenshots are uploaded to the shared folder: 6.9\" and 13\" iPad, light and dark. Can someone check the Spanish captions?" },
  { channel: "launch", author: "seed_priya", minutesAgo: 6, text: "On it after standup." },

  // #design
  { channel: "design", author: "seed_theo", minutesAgo: DAY + 240, text: "New elevation colors: climbs go from sand to rust, descents from sky to deep blue. The old green-to-red read like a heart rate chart." },
  { channel: "design", author: "seed_maya", minutesAgo: DAY + 231, text: "Much easier to read in sunlight. Is the rust still readable on the dark map style?" },
  { channel: "design", author: "seed_theo", minutesAgo: DAY + 226, text: "I bumped its lightness 8% in dark mode. Checked it on a phone outside at noon." },
  { channel: "design", author: "seed_theo", minutesAgo: 95, text: "Offline badge is final: a small filled cloud with a slash, bottom-left of the region card. No more \"Saved\" text label." },
  { channel: "design", author: "seed_jonas", minutesAgo: 88, text: "Support will like that. Half the tickets last month were people not knowing a region was already downloaded." },

  // #general
  { channel: "general", author: "seed_priya", minutesAgo: DAY + 300, text: "Welcome to the team chat. #launch is for the 2.4 release, #design for screens and art." },
  { channel: "general", author: "seed_jonas", minutesAgo: DAY + 75, text: "Trail run Saturday at 8 at the Mill Creek trailhead. 12 km, easy pace, coffee after." },
  { channel: "general", author: "seed_theo", minutesAgo: DAY + 70, text: "In, as long as easy pace means easy pace." },
  { channel: "general", author: "seed_maya", minutesAgo: 30, text: "Standup moved to 10:30 today, Priya is on a call with the App Review team." },
];
