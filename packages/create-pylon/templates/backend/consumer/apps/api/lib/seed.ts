// Demo data for a new app: eight people who post, follow each other, like,
// and comment, so the first launch opens on a feed with content in it.
//
// Photos live in public/images/posts and the server serves them at
// /images/posts/... Demo people have no avatar photo, so the clients draw
// their initial. Demo accounts have no password, so nobody can sign in as
// them. functions/seedDemo.ts writes this data once.

export interface SeedPerson {
	handle: string;
	displayName: string;
	bio: string;
}

export interface SeedPost {
	/** File name under public/images/posts, without `.jpg`. */
	photo: string;
	author: string;
	caption: string;
	/** Hours before the seed runs. */
	age: number;
}

export interface SeedComment {
	photo: string;
	author: string;
	text: string;
	/** Hours after the post. */
	after: number;
}

export const SEED_PEOPLE: SeedPerson[] = [
	{ handle: "maya.rivera", displayName: "Maya Rivera", bio: "Travel photographer. Currently in Lisbon." },
	{ handle: "jordan.eats", displayName: "Jordan Brooks", bio: "Cooking at home, eating out, writing it all down." },
	{ handle: "priya.home", displayName: "Priya Shah", bio: "Interior designer. Small rooms, good light." },
	{ handle: "kenji.mori", displayName: "Kenji Mori", bio: "Architecture and night walks. Tokyo." },
	{ handle: "elena.runs", displayName: "Elena Berg", bio: "Trail runner. Mountains most weekends." },
	{ handle: "sam.and.juno", displayName: "Sam Whitaker", bio: "Juno is the golden one." },
	{ handle: "amara.o", displayName: "Amara Okafor", bio: "Coffee, markets, and early mornings." },
	{ handle: "leo.haddad", displayName: "Leo Haddad", bio: "Road trips and sunsets. Film when I can." },
];

export const SEED_POSTS: SeedPost[] = [
	{ photo: "lisbon", author: "maya.rivera", caption: "Tram 28 at golden hour. Waited twenty minutes for this one and it was worth it.", age: 1.5 },
	{ photo: "coffee", author: "amara.o", caption: "Flat white and a croissant before the market opens.", age: 3 },
	{ photo: "dog", author: "sam.and.juno", caption: "Juno found the tall grass again.", age: 5 },
	{ photo: "run", author: "elena.runs", caption: "Sunrise on the ridge. 18 km, 900 m up, legs gone.", age: 8 },
	{ photo: "ramen", author: "jordan.eats", caption: "Tonkotsu from the new place on 5th. Broth cooked for 14 hours and you can tell.", age: 11 },
	{ photo: "arch", author: "kenji.mori", caption: "Curves and shadows. The museum extension at noon.", age: 14 },
	{ photo: "sunset", author: "leo.haddad", caption: "Big Sur, last light. 30 second exposure.", age: 20 },
];

export const SEED_COMMENTS: SeedComment[] = [
	{ photo: "lisbon", author: "leo.haddad", text: "The light on those tiles. Which street is this?", after: 0.3 },
	{ photo: "lisbon", author: "maya.rivera", text: "Rua da Conceição, just before it turns up to the cathedral.", after: 0.6 },
	{ photo: "lisbon", author: "amara.o", text: "Adding this to the list for May.", after: 0.9 },
	{ photo: "coffee", author: "jordan.eats", text: "That rosetta is perfect.", after: 0.5 },
	{ photo: "coffee", author: "priya.home", text: "Where is this? I need that table.", after: 1 },
	{ photo: "dog", author: "elena.runs", text: "Juno looks so happy.", after: 0.4 },
	{ photo: "dog", author: "amara.o", text: "Best dog on this app.", after: 1.2 },
	{ photo: "run", author: "kenji.mori", text: "Incredible view. How early did you start?", after: 1 },
	{ photo: "run", author: "elena.runs", text: "Headlamps on at 4:30.", after: 1.5 },
	{ photo: "ramen", author: "maya.rivera", text: "Going this week.", after: 2 },
	{ photo: "arch", author: "priya.home", text: "Those shadows are unreal.", after: 1 },
	{ photo: "sunset", author: "sam.and.juno", text: "Colors like this make me want to drive up the coast tonight.", after: 2 },
];

/** Which demo people like which photo. Deterministic, so every seed looks the same. */
export function seedLikers(photo: string): string[] {
	const people = SEED_PEOPLE.map((p) => p.handle);
	let hash = 0;
	for (const ch of photo) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
	return people.filter((_, i) => ((hash >> i) & 3) !== 0);
}

/** Everyone follows everyone else, except a few pairs. */
export function seedFollows(): Array<[string, string]> {
	const out: Array<[string, string]> = [];
	for (const a of SEED_PEOPLE) {
		for (const b of SEED_PEOPLE) {
			if (a.handle === b.handle) continue;
			if ((a.handle.length + b.handle.length) % 5 === 0) continue;
			out.push([a.handle, b.handle]);
		}
	}
	return out;
}

export const photoPath = (photo: string) => `/images/posts/${photo}.jpg`;
