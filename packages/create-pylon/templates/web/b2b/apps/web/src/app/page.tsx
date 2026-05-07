import { pylon } from "@/lib/pylon";
import { OrgPicker } from "./components/OrgPicker";

export const dynamic = "force-dynamic";

type Org = {
	id: string;
	slug: string;
	name: string;
	role: string;
	createdAt: string;
};

export default async function HomePage() {
	const orgs = await pylon
		.json<Org[]>("/api/fn/myOrgs", {
			method: "POST",
			body: "{}",
			headers: { "Content-Type": "application/json" },
		})
		.catch(() => [] as Org[]);

	return (
		<main className="mx-auto max-w-3xl px-6 py-12 space-y-8">
			<header className="space-y-2">
				<h1 className="text-3xl font-semibold tracking-tight">__APP_NAME__</h1>
				<p className="text-sm text-neutral-500 dark:text-neutral-400">
					Multi-tenant SaaS scaffold. Pick an org to manage its members and
					projects, or create a new one. Tenant isolation is enforced
					server-side by Pylon row-level policies — clients can&apos;t leak
					data across orgs even if they fabricate <code className="font-mono text-xs">orgId</code>{" "}
					in a request.
				</p>
			</header>

			<OrgPicker initialOrgs={orgs} />
		</main>
	);
}
