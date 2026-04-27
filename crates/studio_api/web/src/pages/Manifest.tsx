import { Badge } from "@/components/ui/badge";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import {
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "@/components/ui/tabs";
import { MANIFEST } from "@/lib/pylon";

export function ManifestPage() {
	return (
		<div className="space-y-4">
			<div className="grid gap-4 md:grid-cols-4">
				<Stat label="Entities" value={MANIFEST.entities.length} />
				<Stat label="Queries" value={MANIFEST.queries.length} />
				<Stat label="Actions" value={MANIFEST.actions.length} />
				<Stat label="Routes" value={MANIFEST.routes.length} />
			</div>

			<Tabs defaultValue="entities">
				<TabsList>
					<TabsTrigger value="entities">Entities</TabsTrigger>
					<TabsTrigger value="queries">Queries</TabsTrigger>
					<TabsTrigger value="actions">Actions</TabsTrigger>
					<TabsTrigger value="raw">Raw JSON</TabsTrigger>
				</TabsList>

				<TabsContent value="entities" className="space-y-3">
					{MANIFEST.entities.map((e) => (
						<Card key={e.name}>
							<CardHeader>
								<CardTitle className="flex items-center gap-2">
									<code>{e.name}</code>
									{e.crdt && <Badge variant="secondary">CRDT</Badge>}
								</CardTitle>
								<CardDescription>
									{e.fields.length} field{e.fields.length === 1 ? "" : "s"}
									{e.indexes && e.indexes.length > 0
										? ` · ${e.indexes.length} index${e.indexes.length === 1 ? "" : "es"}`
										: ""}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<div className="flex flex-wrap gap-1.5">
									{e.fields.map((f) => (
										<code
											key={f.name}
											className="rounded bg-muted px-2 py-0.5 text-xs"
										>
											{f.name}
											{f.optional && "?"}: {f.type}
										</code>
									))}
								</div>
							</CardContent>
						</Card>
					))}
				</TabsContent>

				<TabsContent value="queries">
					<Card>
						<CardContent className="p-0">
							{MANIFEST.queries.length === 0 ? (
								<div className="py-12 text-center text-sm text-muted-foreground">
									No custom queries.
								</div>
							) : (
								<div className="divide-y">
									{MANIFEST.queries.map((q) => (
										<div key={q.name} className="px-4 py-3">
											<code className="text-sm">{q.name}</code>
											{q.input.length > 0 && (
												<span className="ml-2 text-xs text-muted-foreground">
													({q.input.map((f) => `${f.name}: ${f.type}`).join(", ")})
												</span>
											)}
										</div>
									))}
								</div>
							)}
						</CardContent>
					</Card>
				</TabsContent>

				<TabsContent value="actions">
					<Card>
						<CardContent className="p-0">
							{MANIFEST.actions.length === 0 ? (
								<div className="py-12 text-center text-sm text-muted-foreground">
									No custom actions.
								</div>
							) : (
								<div className="divide-y">
									{MANIFEST.actions.map((a) => (
										<div key={a.name} className="px-4 py-3">
											<code className="text-sm">{a.name}</code>
											{a.input.length > 0 && (
												<span className="ml-2 text-xs text-muted-foreground">
													({a.input.map((f) => `${f.name}: ${f.type}`).join(", ")})
												</span>
											)}
										</div>
									))}
								</div>
							)}
						</CardContent>
					</Card>
				</TabsContent>

				<TabsContent value="raw">
					<Card>
						<CardContent className="p-0">
							<pre className="max-h-[70vh] overflow-auto p-4 text-xs">
								{JSON.stringify(MANIFEST, null, 2)}
							</pre>
						</CardContent>
					</Card>
				</TabsContent>
			</Tabs>
		</div>
	);
}

function Stat({ label, value }: { label: string; value: number }) {
	return (
		<Card>
			<CardContent className="p-4">
				<p className="text-xs text-muted-foreground">{label}</p>
				<p className="mt-1 font-mono text-2xl font-semibold">{value}</p>
			</CardContent>
		</Card>
	);
}
