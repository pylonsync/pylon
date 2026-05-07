"use client";

import { useState, useTransition } from "react";
import { Button, Input, Card, CardHeader, CardContent } from "@__APP_NAME_KEBAB__/ui";

type Org = {
	id: string;
	slug: string;
	name: string;
	role: string;
	createdAt: string;
};

export function OrgPicker({ initialOrgs }: { initialOrgs: Org[] }) {
	const [orgs, setOrgs] = useState(initialOrgs);
	const [activeId, setActiveId] = useState<string | null>(
		initialOrgs[0]?.id ?? null,
	);
	const [creating, setCreating] = useState(false);
	const [name, setName] = useState("");
	const [slug, setSlug] = useState("");
	const [pending, startTransition] = useTransition();
	const [error, setError] = useState<string | null>(null);

	async function create(e: React.FormEvent) {
		e.preventDefault();
		setError(null);
		startTransition(async () => {
			const res = await fetch("/api/fn/createOrg", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ slug, name }),
			});
			if (res.ok) {
				const org = (await res.json()) as Org;
				setOrgs((prev) => [
					{ ...org, role: "owner" },
					...prev,
				]);
				setActiveId(org.id);
				setName("");
				setSlug("");
				setCreating(false);
			} else {
				const body = await res.json().catch(() => ({}));
				setError(body?.message ?? "create failed");
			}
		});
	}

	const active = orgs.find((o) => o.id === activeId) ?? null;

	return (
		<div className="space-y-6">
			<Card>
				<CardHeader>
					<div className="flex items-center justify-between">
						<h2 className="text-sm font-medium">Your organizations</h2>
						<Button
							size="sm"
							variant={creating ? "ghost" : "default"}
							onClick={() => setCreating((v) => !v)}
						>
							{creating ? "Cancel" : "New org"}
						</Button>
					</div>
				</CardHeader>
				<CardContent>
					{creating && (
						<form onSubmit={create} className="grid grid-cols-2 gap-2 mb-4">
							<Input
								placeholder="Org name"
								value={name}
								onChange={(e) => setName(e.target.value)}
								disabled={pending}
								required
							/>
							<Input
								placeholder="acme-corp"
								value={slug}
								onChange={(e) => setSlug(e.target.value.toLowerCase())}
								disabled={pending}
								pattern="[a-z0-9][a-z0-9-]{1,30}[a-z0-9]"
								required
							/>
							<Button
								type="submit"
								variant="primary"
								disabled={pending || !name.trim() || !slug.trim()}
								className="col-span-2"
							>
								{pending ? "Creating…" : "Create org"}
							</Button>
							{error && (
								<p className="text-xs text-red-500 col-span-2">{error}</p>
							)}
						</form>
					)}

					{orgs.length === 0 ? (
						<p className="text-sm text-neutral-500 py-2">
							You&apos;re not in any orgs yet. Create one above to get started.
						</p>
					) : (
						<ul className="divide-y divide-neutral-200 dark:divide-neutral-800 -mx-5">
							{orgs.map((o) => (
								<li
									key={o.id}
									className="flex items-center justify-between px-5 py-2.5 cursor-pointer hover:bg-neutral-50 dark:hover:bg-neutral-900"
									data-active={o.id === activeId || undefined}
									onClick={() => setActiveId(o.id)}
								>
									<div>
										<div className="text-sm font-medium">{o.name}</div>
										<div className="text-xs text-neutral-400 font-mono">
											{o.slug}
										</div>
									</div>
									<span
										className={`text-xs font-mono uppercase tracking-wide px-2 py-0.5 rounded ${
											o.role === "owner"
												? "bg-blue-100 text-blue-700 dark:bg-blue-950 dark:text-blue-300"
												: o.role === "admin"
													? "bg-amber-100 text-amber-700 dark:bg-amber-950 dark:text-amber-300"
													: "bg-neutral-100 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400"
										}`}
									>
										{o.role}
									</span>
								</li>
							))}
						</ul>
					)}
				</CardContent>
			</Card>

			{active && <OrgDetail org={active} />}
		</div>
	);
}

function OrgDetail({ org }: { org: Org }) {
	return (
		<Card>
			<CardHeader>
				<div className="flex items-baseline justify-between">
					<h2 className="text-sm font-medium">
						{org.name}{" "}
						<span className="text-xs text-neutral-400 font-mono ml-1">
							/{org.slug}
						</span>
					</h2>
					<span className="text-xs text-neutral-500">your role: {org.role}</span>
				</div>
			</CardHeader>
			<CardContent className="space-y-4">
				<p className="text-sm text-neutral-500">
					Wire member management and project lists into this panel by calling
					<code className="font-mono text-xs mx-1">/api/fn/orgMembers</code>
					and
					<code className="font-mono text-xs mx-1">/api/fn/orgProjects</code>
					with this org&apos;s id. Both queries enforce membership; you only
					see what your role allows.
				</p>
				<div className="text-xs font-mono text-neutral-400">
					orgId: {org.id}
				</div>
			</CardContent>
		</Card>
	);
}
