import { action, v } from "@pylonsync/functions";

/**
 * Self-perpetuating daily cron that ensures the demo always has a
 * fresh, in-progress timed auction. Visitors landing on the home page
 * see something live regardless of when the seed last ran.
 *
 * Pattern: action runs, does its work, schedules itself for the next
 * tick. Pylon doesn't ship a separate cron primitive — actions own
 * their cadence via ctx.scheduler.runAfter, which makes the loop
 * trivially debuggable (it's just functions calling functions).
 *
 * Bootstrap: call /api/fn/dailyAuctionCron once via admin token after
 * deploy. From there the cron self-perpetuates on a 24h cadence. A
 * machine restart breaks the chain — re-bootstrap if so. (Long term:
 * persist scheduled jobs, which the framework's job queue already
 * supports for non-cron scheduling.)
 *
 * Dedup: refuses to insert a second auction in the same UTC day, so
 * accidental double-bootstraps + dev-mode restarts don't pollute the
 * DB. The `dedupTag` field on Auction stores the YYYY-MM-DD stamp.
 */

const CATALOG = [
	{
		title: "Vintage Leica M3",
		description: "1956 chrome body, working meter, near-mint condition.",
		startingCents: 80_000,
	},
	{
		title: "Hand-bound first edition",
		description:
			"Lord of the Rings, 1954 Allen & Unwin first impression. Light foxing.",
		startingCents: 120_000,
	},
	{
		title: "Mid-century Eames lounge",
		description:
			"Original Herman Miller production, rosewood/black leather. Reupholstered cushions.",
		startingCents: 240_000,
	},
	{
		title: "1972 Patek Philippe Calatrava",
		description: "18k yellow gold, manual wind, original buckle.",
		startingCents: 480_000,
	},
	{
		title: "Art Deco bronze sculpture",
		description:
			"Demêtre Chiparus, signed in the bronze. ~52 cm tall.",
		startingCents: 90_000,
	},
	{
		title: "Le Corbusier LC2 sofa",
		description: "Cassina production, polished chrome frame, white leather.",
		startingCents: 220_000,
	},
	{
		title: "Persian Tabriz silk rug",
		description:
			"Late 19th c., 3.2x2.1m, intricate medallion field.",
		startingCents: 75_000,
	},
	{
		title: "Pair of Tiffany table lamps",
		description:
			"Stained glass shade with dragonfly motif. Both signed at the base.",
		startingCents: 180_000,
	},
	{
		title: "Hasselblad 500C/M with 80mm",
		description:
			"Late-70s body, fresh CLA. Includes A12 back and original strap.",
		startingCents: 140_000,
	},
	{
		title: "Concorde signed crew photo",
		description:
			"Final commercial flight, 2003. Signed by full BA flight crew. Framed.",
		startingCents: 60_000,
	},
];

function dayKey(d: Date): string {
	return d.toISOString().slice(0, 10);
}

function pickN<T>(items: T[], n: number, seed: number): T[] {
	const out: T[] = [];
	const used = new Set<number>();
	let s = seed >>> 0;
	while (out.length < n && used.size < items.length) {
		s = (s * 1664525 + 1013904223) >>> 0;
		const idx = s % items.length;
		if (used.has(idx)) continue;
		used.add(idx);
		out.push(items[idx]);
	}
	return out;
}

const PALETTE = [
	"#8b5cf6",
	"#6366f1",
	"#3b82f6",
	"#06b6d4",
	"#10b981",
	"#84cc16",
	"#eab308",
	"#f97316",
	"#ef4444",
	"#ec4899",
];

function pickColor(seed: string): string {
	let h = 0;
	for (const c of seed) h = (h * 31 + c.charCodeAt(0)) | 0;
	return PALETTE[Math.abs(h) % PALETTE.length];
}

const DAY_MS = 24 * 60 * 60 * 1000;
const AUCTION_DURATION_SECS = 22 * 60 * 60; // 22h window so the next day's auction overlaps slightly
const LOTS_PER_AUCTION = 6;

export default action({
	// Scheduler-driven, but declare guest so a guest-session caller is never
	// rejected by the auth: "user" default.
	auth: "guest",
	args: {},
	async handler(ctx) {
		const now = new Date();
		const today = dayKey(now);

		// Dedup: if today's auction already exists, skip the insert and
		// just reschedule. We tag the daily auctions with `dedupTag` =
		// YYYY-MM-DD so the lookup is a single equality query rather
		// than a temporal range.
		const existing = await ctx.runQuery<Array<{ id: string }>>("findAuctionByTag", {
			tag: `daily:${today}`,
		});
		const skipped = existing.length > 0;

		if (!skipped) {
			// Pick a system "auctioneer" user. The cron needs SOME
			// creator id on the row; use the first User created (the seed
			// admin) so dailies don't pollute a real user's profile. If
			// the DB has no users yet, defer one tick and try again.
			const users = await ctx.runQuery<Array<{ id: string }>>(
				"firstUser",
				{},
			);
			if (users.length === 0) {
				console.warn(
					"[dailyAuctionCron] no users yet — deferring 1h until seed runs",
				);
				await ctx.scheduler.runAfter(60 * 60 * 1000, "dailyAuctionCron", {});
				return { skipped: true, reason: "no users" };
			}
			const creatorId = users[0].id;

			const seed = Number.parseInt(today.replaceAll("-", ""), 10);
			const lots = pickN(CATALOG, LOTS_PER_AUCTION, seed);
			const startsAt = now;
			const endsAt = new Date(now.getTime() + AUCTION_DURATION_SECS * 1000);
			const title = `Daily Sale · ${today}`;
			const auctionId = await ctx.runMutation<string>(
				"insertDailyAuction",
				{
					title,
					description:
						"Curated daily timed auction. New lots every 24 hours; bid before the deadline.",
					creatorId,
					startsAt: startsAt.toISOString(),
					endsAt: endsAt.toISOString(),
					bannerColor: pickColor(title),
					dedupTag: `daily:${today}`,
					lots: lots.map((lot, i) => ({
						title: lot.title,
						description: lot.description,
						imageColor: pickColor(`${title}::${lot.title}::${i}`),
						startingCents: lot.startingCents,
						minIncrementCents: Math.max(100, Math.floor(lot.startingCents * 0.05)),
						endsAt: new Date(
							startsAt.getTime() +
								Math.floor((endsAt.getTime() - startsAt.getTime()) /
									LOTS_PER_AUCTION) *
									(i + 1),
						).toISOString(),
					})),
				},
			);
			await ctx.scheduler.runAfter(0, "startAuction", { auctionId });
			await ctx.scheduler.runAfter(1_000, "sweepTimedLots", { auctionId });
		}

		// Self-reschedule for the next 24h tick. We use a fixed 24h
		// delay (not "next midnight UTC") because the framework's
		// scheduler is interval-based — calendar-wall-clock targeting
		// would require its own drift correction over time.
		await ctx.scheduler.runAfter(DAY_MS, "dailyAuctionCron", {});

		return { skipped, today };
	},
});
