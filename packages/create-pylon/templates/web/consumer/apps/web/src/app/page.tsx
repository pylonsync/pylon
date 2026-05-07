import { Feed } from "./components/Feed";

// Feed subscribes to Post / Profile / Like via db.useQuery — every
// new post + every like from any other client re-renders without
// a fetch. No server-side initial data; the sync engine hydrates
// on first connect.
export default function HomePage() {
	return (
		<main className="mx-auto max-w-2xl px-6 py-12 space-y-8">
			<header className="space-y-2">
				<h1 className="text-3xl font-semibold tracking-tight">__APP_NAME__</h1>
				<p className="text-sm text-neutral-500 dark:text-neutral-400">
					Public feed. Profile / Post / Like — wide-open reads, owner-only
					writes. Live updates via Pylon&apos;s sync engine.
				</p>
			</header>

			<Feed />
		</main>
	);
}
