import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { displayName, useAuth } from "@/auth/AuthContext";
import { API_BASE, MANIFEST } from "@/lib/pylon";

export function SettingsPage() {
	const { me, user } = useAuth();
	const account = displayName(user, me);
	return (
		<div className="grid gap-4 md:grid-cols-2">
			<Card>
				<CardHeader>
					<CardTitle>Session</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3 text-sm">
					<Row label="Signed in as" value={account} />
					<Row
						label="User id"
						value={<code className="text-xs">{me?.user_id ?? "anonymous"}</code>}
					/>
					<Row
						label="Admin"
						value={
							<Badge variant={me?.is_admin ? "default" : "outline"}>
								{me?.is_admin ? "Yes" : "No"}
							</Badge>
						}
					/>
					{me?.tenant_id && (
						<Row
							label="Active tenant"
							value={<code className="text-xs">{me.tenant_id}</code>}
						/>
					)}
					<div className="pt-2">
						<Button variant="outline" size="sm" asChild>
							<a href="/studio/logout">Sign out</a>
						</Button>
					</div>
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>Server</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3 text-sm">
					<Row
						label="API base"
						value={<code className="text-xs">{API_BASE || "(same origin)"}</code>}
					/>
					<Row label="App" value={<code className="text-xs">{MANIFEST.name}</code>} />
					<Row label="Version" value={<code className="text-xs">{MANIFEST.version}</code>} />
					<Row
						label="Manifest version"
						value={
							<code className="text-xs">{MANIFEST.manifest_version}</code>
						}
					/>
				</CardContent>
			</Card>
		</div>
	);
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
	return (
		<div className="flex items-center justify-between gap-4">
			<span className="text-muted-foreground">{label}</span>
			<span>{value}</span>
		</div>
	);
}
