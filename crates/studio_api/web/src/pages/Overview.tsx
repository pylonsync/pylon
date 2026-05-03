import { Activity, Database, FileCode, ShieldCheck } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { MANIFEST } from "@/lib/pylon";

export function OverviewPage() {
	const stats = [
		{ label: "Entities", value: MANIFEST.entities.length, icon: Database },
		{ label: "Routes", value: MANIFEST.routes.length, icon: Activity },
		{ label: "Functions", value: MANIFEST.actions.length, icon: FileCode },
		{ label: "Policies", value: MANIFEST.policies.length, icon: ShieldCheck },
	];
	return (
		<div className="space-y-6">
			<div>
				<h1 className="text-2xl font-semibold tracking-tight">Overview</h1>
				<p className="mt-1 text-sm text-muted-foreground">
					{MANIFEST.name} · v{MANIFEST.version}
				</p>
			</div>

			<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
				{stats.map((s) => {
					const Icon = s.icon;
					return (
						<Card key={s.label}>
							<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
								<CardTitle className="text-sm font-medium text-muted-foreground">
									{s.label}
								</CardTitle>
								<Icon className="size-4 text-muted-foreground" />
							</CardHeader>
							<CardContent>
								<div className="text-2xl font-semibold">{s.value}</div>
							</CardContent>
						</Card>
					);
				})}
			</div>

			<Card>
				<CardHeader>
					<CardTitle className="text-base">Entities</CardTitle>
				</CardHeader>
				<CardContent>
					<div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
						{MANIFEST.entities.map((e) => (
							<div
								key={e.name}
								className="flex items-center justify-between rounded-md border p-3"
							>
								<div className="flex flex-col">
									<span className="text-sm font-medium">{e.name}</span>
									<span className="text-xs text-muted-foreground">
										{e.fields.length} fields
									</span>
								</div>
								{e.crdt && (
									<span className="rounded-md bg-status-blue-bg px-2 py-0.5 text-xs text-status-blue-fg">
										CRDT
									</span>
								)}
							</div>
						))}
					</div>
				</CardContent>
			</Card>
		</div>
	);
}
