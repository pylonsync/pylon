import React, { Suspense, use } from "react";
import type { PageProps } from "@pylonsync/react";

export const metadata = {
	title: "__APP_NAME__",
	description: "Share photos with the people you follow.",
};

interface Post {
	id: string;
	imageUrl: string;
	caption?: string | null;
	createdAt: string;
}

/**
 * The public web page for the API host: the app name and the latest
 * photos. The iOS and Expo apps are the real clients.
 */
export default function Page({ serverData }: PageProps) {
	return (
		<main style={{ maxWidth: 960, margin: "0 auto", padding: "48px 16px" }}>
			<h1 style={{ fontSize: 40, fontWeight: 700, letterSpacing: "-0.02em", margin: 0 }}>
				__APP_NAME__
			</h1>
			<p style={{ color: "var(--muted)", margin: "8px 0 32px" }}>
				Share photos with the people you follow. Get the app on iPhone.
			</p>
			<Suspense fallback={null}>
				<Latest promise={serverData.list<Post>("Post")} />
			</Suspense>
		</main>
	);
}

function Latest({ promise }: { promise: Promise<Post[]> }) {
	const posts = use(promise)
		.slice()
		.sort((a, b) => b.createdAt.localeCompare(a.createdAt))
		.slice(0, 12);
	return (
		<div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
			{posts.map((post) => (
				<img
					key={post.id}
					src={post.imageUrl}
					alt={post.caption ?? ""}
					loading="lazy"
					style={{ width: "100%", aspectRatio: "1", objectFit: "cover", display: "block" }}
				/>
			))}
		</div>
	);
}
